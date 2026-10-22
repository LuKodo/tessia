pub mod model;
pub mod service;

#[cfg(test)]
pub(crate) mod test_support {
    //! Builds small real `.xlsx` workbooks for tests, in the layout of the real
    //! Nota Técnica (sheet `Hoja1`, header on row 1).

    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    pub(crate) const REAL_HEADER: [&str; 10] = [
        "MED-CUM",
        "Referencia",
        "Notas",
        "COD GEN",
        "FORMA FARMACEUTICA RIPS",
        "CONCENTRACION MEDICAMENTO RIPS",
        "UNIDAD MEDIDA RIPS",
        "TIPO PRODUCTO",
        "IDMUTUAL",
        "PRECIO CONTRATO",
    ];

    /// Header plus `count` valid rows whose CUM is `<tag>-<n>`.
    pub(crate) fn sample_rows(count: usize, tag: &str) -> Vec<Vec<String>> {
        let header = REAL_HEADER.iter().map(|h| (*h).to_owned()).collect();
        let body = (0..count).map(|n| {
            [
                format!("{tag}-{n}"),
                "ref".into(),
                String::new(),
                format!("G{n}"),
                "TABLETA".into(),
                "500 MG".into(),
                "UN".into(),
                "POS".into(),
                "M1".into(),
                format!("{}", 1000 + n),
            ]
            .to_vec()
        });
        std::iter::once(header).chain(body).collect()
    }

    pub(crate) fn write_sample_nt(path: &Path, count: usize, tag: &str) {
        write_xlsx(path, &sample_rows(count, tag));
    }

    /// Writes `rows` as sheet `Hoja1` (empty strings become empty cells).
    pub(crate) fn write_xlsx(path: &Path, rows: &[Vec<String>]) {
        let mut sheet = String::from(
            "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
        );
        for (r, row) in rows.iter().enumerate() {
            sheet.push_str(&format!("<row r=\"{}\">", r + 1));
            for (c, text) in row.iter().enumerate().filter(|(_, t)| !t.is_empty()) {
                let column = char::from(b'A' + c as u8);
                sheet.push_str(&format!(
                    "<c r=\"{column}{}\" t=\"inlineStr\"><is><t>{text}</t></is></c>",
                    r + 1
                ));
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>");

        let parts: [(&str, &str); 5] = [
            (
                "[Content_Types].xml",
                "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>",
            ),
            (
                "_rels/.rels",
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>",
            ),
            (
                "xl/workbook.xml",
                "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Hoja1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
            ),
            (
                "xl/_rels/workbook.xml.rels",
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
            ),
            ("xl/worksheets/sheet1.xml", &sheet),
        ];
        let file = File::create(path).expect("create xlsx");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, content) in parts {
            zip.start_file(name, options).expect("start zip entry");
            zip.write_all(content.as_bytes()).expect("write zip entry");
        }
        zip.finish().expect("finish zip");
    }
}
