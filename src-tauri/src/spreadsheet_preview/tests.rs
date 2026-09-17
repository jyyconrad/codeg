use std::io::Write;
use std::path::Path;

use super::{read_preview, PreviewQuery};

fn query(row_offset: u32, row_limit: u32, col_offset: u32, col_limit: u32) -> PreviewQuery {
    PreviewQuery {
        sheet: None,
        row_offset,
        row_limit,
        col_offset,
        col_limit,
    }
}

fn write_csv(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write csv");
    path
}

#[test]
fn csv_pages_header_plus_200_data_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::from("h1,h2\n");
    for i in 1..=200 {
        body.push_str(&format!("r{i},v{i}\n"));
    }
    let path = write_csv(dir.path(), "data.csv", &body);

    let first = read_preview(&path, query(0, 100, 0, 64)).unwrap();
    assert_eq!(first.header, vec!["h1", "h2"]);
    assert_eq!(first.rows.len(), 100);
    assert_eq!(first.rows[0], vec!["r1", "v1"]);
    assert_eq!(first.total_rows, 201);
    assert!(!first.eof);

    let second = read_preview(&path, query(100, 100, 0, 64)).unwrap();
    assert_eq!(second.rows.len(), 100);
    assert_eq!(second.rows[0], vec!["r101", "v101"]);
    assert!(second.eof);
}

#[test]
fn csv_column_window() {
    let dir = tempfile::tempdir().unwrap();
    let header: Vec<String> = (1..=80).map(|i| format!("c{i}")).collect();
    let data: Vec<String> = (1..=80).map(|i| format!("v{i}")).collect();
    let body = format!("{}\n{}\n", header.join(","), data.join(","));
    let path = write_csv(dir.path(), "wide.csv", &body);

    let page = read_preview(&path, query(0, 100, 64, 64)).unwrap();
    assert_eq!(page.header.len(), 16);
    assert_eq!(page.header[0], "c65");
    assert_eq!(page.rows[0][0], "v65");
    assert_eq!(page.total_columns, 80);
}

#[test]
fn csv_keeps_leading_zeros() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_csv(dir.path(), "z.csv", "id,code\n1,001\n");
    let page = read_preview(&path, query(0, 100, 0, 64)).unwrap();
    assert_eq!(page.rows[0][1], "001");
}

#[test]
fn csv_decodes_gb18030() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cn.csv");
    let (encoded, _, _) = encoding_rs::GB18030.encode("姓名,值\n张三,1\n");
    std::fs::write(&path, encoded.as_ref()).unwrap();
    let page = read_preview(&path, query(0, 100, 0, 64)).unwrap();
    assert_eq!(page.header[0], "姓名");
    assert_eq!(page.rows[0][0], "张三");
}

#[test]
fn csv_header_only_is_eof_with_empty_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_csv(dir.path(), "h.csv", "a,b\n");
    let page = read_preview(&path, query(0, 100, 0, 64)).unwrap();
    assert_eq!(page.total_rows, 1);
    assert!(page.rows.is_empty());
    assert!(page.eof);
}

#[test]
fn csv_row_offset_past_end_is_eof_not_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_csv(dir.path(), "s.csv", "a\n1\n");
    let page = read_preview(&path, query(50, 100, 0, 64)).unwrap();
    assert!(page.rows.is_empty());
    assert!(page.eof);
}

#[test]
fn xlsx_reads_cached_formula_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.xlsx");
    write_formula_xlsx(&path);
    let page = read_preview(&path, query(0, 100, 0, 64)).unwrap();
    assert_eq!(page.header[0], "n");
    assert_eq!(page.rows[0][0], "2");
}

#[test]
fn xlsx_unknown_sheet_is_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.xlsx");
    write_formula_xlsx(&path);
    let err = read_preview(
        &path,
        PreviewQuery {
            sheet: Some("nope".into()),
            row_offset: 0,
            row_limit: 100,
            col_offset: 0,
            col_limit: 64,
        },
    )
    .unwrap_err();
    assert!(err.message.to_lowercase().contains("sheet"));
}

fn write_formula_xlsx(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    zip.start_file("[Content_Types].xml", opts).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
    )
    .unwrap();

    zip.start_file("_rels/.rels", opts).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
    )
    .unwrap();

    zip.start_file("xl/workbook.xml", opts).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
    )
    .unwrap();

    zip.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
    )
    .unwrap();

    zip.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>n</t></is></c></row>
<row r="2"><c r="A2"><f>1+1</f><v>2</v></c></row>
</sheetData>
</worksheet>"#,
    )
    .unwrap();

    zip.finish().unwrap();
}
