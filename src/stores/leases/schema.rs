diesel::table! {
    leases (name) {
        name -> Text,
        owner -> Text,
        expires_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}
