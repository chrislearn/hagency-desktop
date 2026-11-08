use hagency_matrix_format::MatrixContent;
use serde_json::Value;

#[test]
fn native_matrix_format_vectors() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../fixtures/matrix-format.json")).unwrap();
    let mut differences = Vec::new();
    for vector in fixture["vectors"].as_array().unwrap() {
        let input = MatrixContent::new(vector["input"].clone()).unwrap();
        let actual = input.formatted().unwrap().into_value();
        if actual != vector["expected"] {
            differences.push(format!(
                "{}:\nactual: {}\nexpected: {}",
                vector["name"], actual, vector["expected"]
            ));
        }
        assert_eq!(input.as_value(), &vector["input"]);
    }
    assert!(differences.is_empty(), "{}", differences.join("\n\n"));
}
