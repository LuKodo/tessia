//! Structure of the rendered pages: page count and size, embedded images,
//! placement and drawn text.

#[path = "fixtures/soporte/common.rs"]
mod soporte_common;

use image::ImageFormat;
use lopdf::{Dictionary, Document, Object};
use soporte_common::*;
use tessia::phases::soporte::render::delivery::{self, DeliveryRow};
use tessia::phases::soporte::render::{formula, merge, signature};

fn count_ops(page: &[(String, Vec<Object>)], operator: &str) -> usize {
    page.iter().filter(|(op, _)| op == operator).count()
}

#[test]
fn formula_page_is_letter_with_the_image_fitted_at_25_26() {
    // Wider than tall: limited by the 561 pt width.
    let pdf = formula::render(&rgb_image(1122, 400, ImageFormat::Png)).expect("render");
    assert_eq!(merge::page_count(&pdf).expect("pages"), 1);
    assert_eq!(merge::page_sizes(&pdf).expect("sizes"), [(612.0, 792.0)]);
    assert_eq!(merge::page_image_counts(&pdf).expect("images"), [1]);
    let ops = &page_operations(&pdf)[0];
    let cm = ops.iter().find(|(op, _)| op == "cm").expect("cm");
    let m: Vec<f64> = cm.1.iter().map(number).collect();
    assert!((m[0] - 561.0).abs() < 1e-3, "width {}", m[0]);
    assert!((m[3] - 200.0).abs() < 1e-3, "height {}", m[3]);
    assert!((m[4] - 25.0).abs() < 1e-3, "x {}", m[4]);
    assert!((m[5] - (792.0 - 26.0 - 200.0)).abs() < 1e-3, "y {}", m[5]);
}

#[test]
fn taller_images_are_limited_by_the_739_pt_height() {
    let pdf = formula::render(&rgb_image(300, 1478, ImageFormat::Jpeg)).expect("render");
    let ops = &page_operations(&pdf)[0];
    let cm = ops.iter().find(|(op, _)| op == "cm").expect("cm");
    let m: Vec<f64> = cm.1.iter().map(number).collect();
    assert!((m[3] - 739.0).abs() < 1e-3);
    assert!((m[0] - 150.0).abs() < 1e-3);
}

#[test]
fn every_supported_format_gives_one_page_with_one_image() {
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
    ] {
        let pdf = formula::render(&rgb_image(64, 48, format)).expect("render");
        assert_eq!(merge::page_count(&pdf).expect("pages"), 1, "{format:?}");
        assert_eq!(
            merge::page_image_counts(&pdf).expect("images"),
            [1],
            "{format:?}"
        );
    }
}

#[test]
fn jpeg_is_kept_and_gif_bmp_tiff_are_normalized_to_lossless_pixels() {
    let doc_of =
        |data: &[u8]| Document::load_mem(&formula::render(data).expect("render")).expect("load");
    let filter_of = |doc: &Document| -> Vec<u8> {
        let page = *doc.get_pages().values().next().expect("page");
        let images = doc.get_page_images(page).expect("images");
        images[0]
            .filters
            .clone()
            .expect("filters")
            .remove(0)
            .into_bytes()
    };
    assert_eq!(
        filter_of(&doc_of(&rgb_image(32, 32, ImageFormat::Jpeg))),
        b"DCTDecode"
    );
    for format in [
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::Png,
    ] {
        assert_eq!(
            filter_of(&doc_of(&rgb_image(32, 32, format))),
            b"FlateDecode",
            "{format:?}"
        );
    }
}

#[test]
fn invalid_formula_images_are_errors() {
    assert!(formula::render(b"garbage").is_err());
    let mut truncated = rgb_image(100, 100, ImageFormat::Png);
    truncated.truncate(truncated.len() / 2);
    assert!(formula::render(&truncated).is_err());
}

fn rows(n: usize) -> Vec<DeliveryRow> {
    (1..=n)
        .map(|i| DeliveryRow {
            incremento: i,
            invoice: "FEV1".into(),
            date: "2026-10-05".into(),
            authorization: "AUT-1".into(),
            name: "ANA PEREZ".into(),
            doc_type: "CC".into(),
            doc_number: "123".into(),
            article: format!("MEDICAMENTO {i}"),
            quantity: "2".into(),
        })
        .collect()
}

fn logo() -> Vec<u8> {
    rgb_image(300, 100, ImageFormat::Png)
}

fn sig() -> Vec<u8> {
    signature::render("ANA PEREZ").expect("signature")
}

#[test]
fn delivery_pages_hold_18_rows_then_21() {
    for (n, pages) in [(1, 1), (18, 1), (19, 2), (39, 2), (40, 3)] {
        let pdf = delivery::render(&rows(n), &logo(), &sig()).expect("render");
        assert_eq!(merge::page_count(&pdf).expect("pages"), pages, "{n} rows");
        assert!(
            merge::page_sizes(&pdf)
                .expect("sizes")
                .iter()
                .all(|s| *s == (612.0, 792.0))
        );
    }
}

#[test]
fn delivery_page_draws_the_header_rows_footer_and_images() {
    let pdf = delivery::render(&rows(19), &logo(), &sig()).expect("render");
    let texts = merge::page_texts(&pdf).expect("texts");
    assert!(
        texts[0]
            .contains("CONSTANCIA DE ENTREGA Y RECIBIDO DE MEDICAMENTOS Y/O DISPOSITIVOS MEDICOS")
    );
    assert!(
        !texts[1].contains("CONSTANCIA DE ENTREGA"),
        "the title band is on the first page only"
    );
    for page in &texts {
        assert!(page.contains("ANA PEREZ TIPO DOC: CC  NUM DOC: 123"));
        assert!(page.contains("05/10/2026"));
        assert!(page.contains("AUT-1"));
        assert!(page.contains("NOMBRE DEL AFILIADO"));
        assert!(page.contains("Certifico la recepci\u{f3}n conforme"));
    }
    assert!(texts[0].contains("MEDICAMENTO 18") && !texts[0].contains("MEDICAMENTO 19"));
    assert!(texts[1].contains("MEDICAMENTO 19"));
    let ops = page_operations(&pdf);
    // Logo + signature on the first page, signature alone on the second.
    assert_eq!(count_ops(&ops[0], "Do"), 2);
    assert_eq!(count_ops(&ops[1], "Do"), 1);
    // One band (two boxes of four segments) per row, plus header and footer boxes.
    assert!(count_ops(&ops[0], "S") > count_ops(&ops[1], "S"));
}

#[test]
fn delivery_quantities_round_half_even_and_missing_values_print_null() {
    let mut r = rows(3);
    r[0].quantity = "2,5".into();
    r[1].quantity = "3.5".into();
    r[2].quantity = String::new();
    r[2].authorization = String::new();
    let pdf = delivery::render(&r, &logo(), &sig()).expect("render");
    let text = &merge::page_texts(&pdf).expect("texts")[0];
    assert!(
        text.contains("MEDICAMENTO 1 2 MEDICAMENTO 2 4 MEDICAMENTO 3 null"),
        "{text}"
    );
}

#[test]
fn delivery_rejects_bad_input() {
    assert!(delivery::render(&[], &logo(), &sig()).is_err());
    let mut bad = rows(1);
    bad[0].date = "05/10/2026".into();
    assert!(delivery::render(&bad, &logo(), &sig()).is_err());
    let mut bad = rows(1);
    bad[0].quantity = "x".into();
    assert!(delivery::render(&bad, &logo(), &sig()).is_err());
    assert!(delivery::render(&rows(1), b"nope", &sig()).is_err());
    assert!(delivery::render(&rows(1), &logo(), b"").is_err());
    let gif = rgb_image(8, 8, ImageFormat::Gif);
    assert!(
        delivery::render(&rows(1), &gif, &sig()).is_err(),
        "only PNG/JPEG for the logo"
    );
}

#[test]
fn long_articles_show_only_their_first_wrapped_line() {
    let mut r = rows(1);
    r[0].article = "PALABRA ".repeat(60);
    let pdf = delivery::render(&r, &logo(), &sig()).expect("render");
    let text = &merge::page_texts(&pdf).expect("texts")[0];
    let shown = text.matches("PALABRA").count();
    assert!(shown > 5 && shown < 60, "{shown}");
}

#[test]
fn merge_concatenates_pages_in_order_and_keeps_their_text() {
    let a = text_pdf("A", 2);
    let b = text_pdf("B", 1);
    let c = formula::render(&rgb_image(100, 100, ImageFormat::Png)).expect("formula");
    let merged = merge::merge(&[&a, &b, &c]).expect("merge");
    assert_eq!(merge::page_count(&merged).expect("pages"), 4);
    let texts = merge::page_texts(&merged).expect("texts");
    assert_eq!(&texts[..3], ["A page 1", "A page 2", "B page 1"]);
    assert_eq!(
        merge::page_image_counts(&merged).expect("images"),
        [0, 0, 0, 1]
    );
}

#[test]
fn merge_flattens_inherited_page_attributes() {
    // A page without MediaBox/Resources that inherits both from its Pages node.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set("Parent", Object::Reference(pages_id));
    let page_id = doc.add_object(page);
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    pages.set(
        "MediaBox",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(200),
            Object::Integer(300),
        ]),
    );
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let root = doc.add_object(catalog);
    doc.trailer.set("Root", Object::Reference(root));
    let mut inherited = Vec::new();
    doc.save_to(&mut inherited).expect("save");

    let merged = merge::merge(&[&text_pdf("A", 1), &inherited]).expect("merge");
    assert_eq!(
        merge::page_sizes(&merged).expect("sizes"),
        [(612.0, 792.0), (200.0, 300.0)]
    );
}

#[test]
fn merge_rejects_unreadable_parts() {
    let err = merge::merge(&[&text_pdf("A", 1), b"not a pdf"]).expect_err("garbage");
    assert!(format!("{err:#}").contains("part 2"), "{err:#}");
    assert!(merge::merge(&[]).is_err());
}

#[test]
fn merged_documents_survive_a_second_merge() {
    let once = merge::merge(&[&text_pdf("A", 2), &text_pdf("B", 1)]).expect("merge");
    let twice = merge::merge(&[&once, &text_pdf("C", 1)]).expect("merge");
    assert_eq!(
        merge::page_texts(&twice).expect("texts"),
        ["A page 1", "A page 2", "B page 1", "C page 1"]
    );
}

#[test]
fn gray_and_alpha_images_embed_with_the_right_color_space() {
    let pdf = formula::render(&gray_image(16, 16, ImageFormat::Png)).expect("gray");
    assert_eq!(merge::page_image_counts(&pdf).expect("images"), [1]);
    let r = delivery::render(&rows(1), &logo(), &rgba_image(40, 10)).expect("alpha signature");
    assert_eq!(merge::page_count(&r).expect("pages"), 1);
}
