use calamine::{CellErrorType, Data, ExcelDateTime};

pub fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(n) => n.to_string(),
        Data::Float(n) => format_float(*n),
        Data::Bool(true) => "TRUE".to_string(),
        Data::Bool(false) => "FALSE".to_string(),
        Data::DateTime(dt) => format_excel_datetime(dt),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(err) => format_error(err),
    }
}

pub fn format_float(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let formatted = format!("{value:.10}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn format_excel_datetime(dt: &ExcelDateTime) -> String {
    match dt.as_datetime() {
        Some(ndt) => {
            if ndt.time() == chrono::NaiveTime::MIN {
                ndt.format("%Y-%m-%d").to_string()
            } else {
                ndt.format("%Y-%m-%d %H:%M:%S").to_string()
            }
        }
        None => dt.to_string(),
    }
}

fn format_error(err: &CellErrorType) -> String {
    match err {
        CellErrorType::Div0 => "#DIV/0!".to_string(),
        CellErrorType::NA => "#N/A".to_string(),
        CellErrorType::Name => "#NAME?".to_string(),
        CellErrorType::Null => "#NULL!".to_string(),
        CellErrorType::Num => "#NUM!".to_string(),
        CellErrorType::Ref => "#REF!".to_string(),
        CellErrorType::Value => "#VALUE!".to_string(),
        CellErrorType::GettingData => "#GETTING_DATA".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_and_trimmed_floats() {
        assert_eq!(format_cell(&Data::Int(42)), "42");
        assert_eq!(format_cell(&Data::Float(2.0)), "2");
        assert_eq!(format_cell(&Data::Float(1.5)), "1.5");
        assert_eq!(format_cell(&Data::Float(0.0)), "0");
    }

    #[test]
    fn bools_and_errors() {
        assert_eq!(format_cell(&Data::Bool(true)), "TRUE");
        assert_eq!(format_cell(&Data::Bool(false)), "FALSE");
        assert_eq!(format_cell(&Data::Error(CellErrorType::Div0)), "#DIV/0!");
    }

    #[test]
    fn empty_and_text() {
        assert_eq!(format_cell(&Data::Empty), "");
        assert_eq!(format_cell(&Data::String("001".into())), "001");
    }
}
