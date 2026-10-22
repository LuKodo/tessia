//! The rights validator client against a mock Keycloak + FHIR server.

#[path = "fixtures/derechos/support.rs"]
mod support;

use std::sync::Arc;

use tessia::http::RetryPolicy;
use tessia::phases::derechos::client::{
    ClientConfig, ValidatorClient, ValidatorError, shared_client,
};
use tessia::phases::derechos::rights::{AffiliateQuery, Rights};
use tessia::testing::{MockResponse, MockServer};

use support::{complete_bundle, server, settings, token_requests, validate_requests};

fn instant_retry() -> RetryPolicy {
    let mut policy = RetryPolicy::instant(4);
    policy.retry_non_idempotent = true;
    policy
}

fn client_for(mock: &MockServer) -> ValidatorClient {
    let config = ClientConfig::from_settings(&settings(mock)).expect("config");
    ValidatorClient::with_retry(config, 8, instant_retry()).expect("client")
}

fn affiliate(doc_type: &str, number: &str) -> AffiliateQuery {
    AffiliateQuery::normalized(doc_type, number)
}

#[test]
fn it_logs_in_with_the_password_grant_and_sends_the_fhir_parameters() {
    let mock = server(3600, |_| MockResponse::json(200, &complete_bundle()));
    let client = client_for(&mock);

    let lookup = client.validate(&affiliate("CC", "123")).expect("validate");

    assert_eq!(
        lookup.rights,
        Some(Rights {
            user_type: "04".into(),
            zone: "U".into(),
            dane: "13001".into(),
            sex: "F".into(),
            birth_date: "1990-01-31".into(),
        })
    );
    assert_eq!(lookup.queried_doc_type, "CC");
    let requests = mock.requests();
    let token = &requests[0];
    assert_eq!(
        (token.method.as_str(), token.path.as_str()),
        ("POST", "/token")
    );
    assert_eq!(
        token.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        token.body_text(),
        "grant_type=password&client_id=client&client_secret=secret&username=user&password=pass"
    );
    let validate = &requests[1];
    assert_eq!(validate.method, "POST");
    assert_eq!(validate.header("authorization"), Some("Bearer t1"));
    assert_eq!(validate.header("content-type"), Some("application/json"));
    assert_eq!(validate.header("accept"), Some("*/*"));
    assert_eq!(
        validate.body_text(),
        r#"{"resourceType":"Parameters","id":"CorrelationId","parameter":[{"name":"documentType","valueString":"CC"},{"name":"documentId","valueString":"123"}]}"#
    );
}

#[test]
fn the_token_is_cached_until_thirty_seconds_before_it_expires() {
    let mock = server(3600, |_| MockResponse::json(200, &complete_bundle()));
    let client = client_for(&mock);
    for n in 0..3 {
        client
            .validate(&affiliate("CC", &n.to_string()))
            .expect("validate");
    }
    assert_eq!(token_requests(&mock), 1);

    // expires_in 20 s is inside the 30 s early-expiry window: never reused.
    let short = server(20, |_| MockResponse::json(200, &complete_bundle()));
    let client = client_for(&short);
    for n in 0..3 {
        client
            .validate(&affiliate("CC", &n.to_string()))
            .expect("validate");
    }
    assert_eq!(token_requests(&short), 3);
}

#[test]
fn document_types_fall_back_on_400_to_410() {
    let mock = server(3600, |q| {
        match (q.doc_type.as_str(), q.doc_number.as_str()) {
            ("TI", _) | ("RC", _) => MockResponse::text(400, "bad"),
            (_, "1") => MockResponse::json(200, &complete_bundle()),
            _ => MockResponse::text(410, "gone"),
        }
    });
    let client = client_for(&mock);

    let ti = client.validate(&affiliate("TI", "1")).expect("ti");
    assert_eq!(ti.queried_doc_type, "CC");
    assert!(ti.rights.is_some());
    let kinds: Vec<_> = validate_requests(&mock)
        .iter()
        .map(|r| support_doc_type(&r.body))
        .collect();
    assert_eq!(kinds, ["TI", "CC"]);

    let rc = client.validate(&affiliate("RC", "1")).expect("rc");
    assert_eq!(rc.queried_doc_type, "CC");
    let kinds: Vec<_> = validate_requests(&mock)
        .iter()
        .map(|r| support_doc_type(&r.body))
        .collect();
    assert_eq!(kinds[2..], ["RC", "TI", "CC"]);

    // CC has no fallback: 4xx counts as not found after one request.
    let before = validate_requests(&mock).len();
    let cc = client.validate(&affiliate("CC", "2")).expect("cc");
    assert!(cc.rights.is_none());
    assert_eq!(validate_requests(&mock).len(), before + 1);

    // After exhausting the chain the answer is "not found" too.
    let gone = client.validate(&affiliate("RC", "9")).expect("rc 9");
    assert!(gone.rights.is_none());
    assert_eq!(gone.queried_doc_type, "CC");
}

fn support_doc_type(body: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(body).expect("json");
    value["parameter"][0]["valueString"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_empty_bundle_is_not_found_and_garbage_is_technical() {
    let mock = server(3600, |q| match q.doc_number.as_str() {
        "empty" => MockResponse::json(200, r#"{"entry":[]}"#),
        _ => MockResponse::text(200, "<html>"),
    });
    let client = client_for(&mock);
    assert_eq!(
        client
            .validate(&affiliate("CC", "empty"))
            .expect("empty")
            .rights,
        None
    );
    let err = client.validate(&affiliate("CC", "x")).expect_err("garbage");
    assert!(matches!(err, ValidatorError::Technical(_)), "{err:?}");
    assert_eq!(
        validate_requests(&mock).len(),
        2,
        "invalid JSON is not retried"
    );
}

#[test]
fn server_errors_and_throttling_are_retried_four_times() {
    let mock = server(3600, |_| MockResponse::text(503, "down"));
    let client = client_for(&mock);
    let err = client.validate(&affiliate("CC", "1")).expect_err("5xx");
    assert!(
        matches!(&err, ValidatorError::Technical(m) if m.contains("503")),
        "{err:?}"
    );
    assert_eq!(validate_requests(&mock).len(), 4);

    let throttled = server(3600, |_| MockResponse::text(429, "slow down"));
    let client = client_for(&throttled);
    assert!(matches!(
        client.validate(&affiliate("CC", "1")),
        Err(ValidatorError::Technical(_))
    ));
    assert_eq!(validate_requests(&throttled).len(), 4);
}

#[test]
fn a_transient_failure_followed_by_success_is_recovered() {
    let calls = support::shared_counter();
    let seen = calls.clone();
    let mock = server(3600, move |_| {
        if seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
            MockResponse::text(502, "bad gateway")
        } else {
            MockResponse::json(200, &complete_bundle())
        }
    });
    let client = client_for(&mock);
    assert!(
        client
            .validate(&affiliate("CC", "1"))
            .expect("recovered")
            .rights
            .is_some()
    );
    assert_eq!(validate_requests(&mock).len(), 3);
}

#[test]
fn a_401_renews_the_token_once_and_resends() {
    let mock = server(3600, |q| {
        if q.bearer == "t2" {
            MockResponse::json(200, &complete_bundle())
        } else {
            MockResponse::text(401, "expired")
        }
    });
    let client = client_for(&mock);
    let lookup = client.validate(&affiliate("CC", "1")).expect("renewed");
    assert!(lookup.rights.is_some());
    assert_eq!(token_requests(&mock), 2);
    let bearers: Vec<_> = validate_requests(&mock)
        .iter()
        .map(|r| r.header("authorization").unwrap_or_default().to_owned())
        .collect();
    assert_eq!(bearers, ["Bearer t1", "Bearer t2"]);
}

#[test]
fn concurrent_401s_cause_one_renewal_not_one_per_worker() {
    let mock = server(3600, |q| {
        if q.bearer == "t2" {
            MockResponse::json(200, &complete_bundle())
        } else {
            MockResponse::text(401, "expired")
        }
    });
    let client = Arc::new(client_for(&mock));
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let client = client.clone();
            std::thread::spawn(move || client.validate(&affiliate("CC", &n.to_string())))
        })
        .collect();
    for handle in handles {
        let lookup = handle.join().expect("worker").expect("validated");
        assert!(lookup.rights.is_some());
    }
    assert_eq!(token_requests(&mock), 2, "one login plus one renewal");
}

#[test]
fn a_token_that_is_still_rejected_is_an_authentication_error() {
    let mock = server(3600, |_| MockResponse::text(401, "no"));
    let client = client_for(&mock);
    let err = client
        .validate(&affiliate("CC", "1"))
        .expect_err("rejected");
    assert!(matches!(err, ValidatorError::Authentication(_)), "{err:?}");
    assert_eq!(token_requests(&mock), 2);
    assert_eq!(validate_requests(&mock).len(), 2, "resent exactly once");
}

#[test]
fn a_403_is_an_authentication_error_without_renewal() {
    let mock = server(3600, |_| MockResponse::text(403, "forbidden"));
    let client = client_for(&mock);
    let err = client
        .validate(&affiliate("CC", "1"))
        .expect_err("forbidden");
    assert!(matches!(err, ValidatorError::Authentication(_)), "{err:?}");
    assert_eq!(token_requests(&mock), 1);
    assert_eq!(validate_requests(&mock).len(), 1);
}

#[test]
fn a_failed_login_is_an_authentication_error_and_never_leaks_credentials() {
    for token_response in [
        MockResponse::text(401, "bad credentials"),
        MockResponse::json(200, r#"{"expires_in":60}"#),
        MockResponse::text(200, "not json"),
    ] {
        let mock = MockServer::start(move |req| {
            if req.path == "/token" {
                token_response.clone()
            } else {
                MockResponse::json(200, &complete_bundle())
            }
        });
        let client = client_for(&mock);
        let err = client
            .validate(&affiliate("CC", "1"))
            .expect_err("no token");
        match &err {
            ValidatorError::Authentication(message) => {
                assert!(
                    !message.contains("secret") && !message.contains("pass"),
                    "{message}"
                );
            }
            other => panic!("expected an authentication error, got {other:?}"),
        }
        assert!(validate_requests(&mock).is_empty());
    }
}

#[test]
fn the_process_shares_one_client_per_configuration() {
    let mock = server(3600, |_| MockResponse::json(200, &complete_bundle()));
    let config = ClientConfig::from_settings(&settings(&mock)).expect("config");
    let first = shared_client(&config, 30).expect("first");
    let second = shared_client(&config, 5).expect("second");
    assert!(Arc::ptr_eq(&first, &second));
    first.validate(&affiliate("CC", "1")).expect("validate");
    second.validate(&affiliate("CC", "2")).expect("validate");
    assert_eq!(token_requests(&mock), 1, "one login for every caller");
    assert_eq!(
        first.limiter().max(),
        30,
        "the ceiling of the call that built it"
    );

    let other = server(3600, |_| MockResponse::json(200, &complete_bundle()));
    let other_config = ClientConfig::from_settings(&settings(&other)).expect("config");
    let third = shared_client(&other_config, 8).expect("third");
    assert!(!Arc::ptr_eq(&first, &third));
}

#[test]
fn missing_configuration_names_the_keys_and_never_values() {
    let incomplete = tessia::config::Settings::default().with("derechos.client_secret", "hunter2");
    let err = ClientConfig::from_settings(&incomplete).expect_err("missing keys");
    let text = format!("{err:#}");
    assert!(text.contains("derechos.token_url") && text.contains("TESSIA_DERECHOS_TOKEN_URL"));
    assert!(!text.contains("hunter2"));
}
