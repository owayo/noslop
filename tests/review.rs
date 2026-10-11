use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};

#[test]
fn cli_review_verifies_absence_and_citations_then_rejects_changed_source() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("guide.md");
    let profile = dir.path().join("reader.json");
    let response_file = dir.path().join("response.json");
    let source = "# 申込み\n\n申込みを受け付けます。\n";
    std::fs::write(&file, source).unwrap();
    std::fs::write(&profile, r#"{"purpose":"申込み案内","audience":"初めて申し込む人","requiredInformation":["受付期限"]}"#).unwrap();
    let output = cargo_bin_cmd!("noslop")
        .args([
            "review",
            file.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let request: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(request["document"]["name"], "guide.md");
    let observations: Vec<_> = request["techniques"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            if t["id"] == "purpose_coverage" {
                json!({ "id": t["id"], "status": "needs_review",
            "reason": "全文に期限がありません", "readerImpact": "締切を判断できません",
            "question": "期限を含めますか", "evidence": [] })
            } else if t["id"] == "headings" {
                json!({ "id": t["id"], "status": "clear",
            "reason": "申込みの案内に対応する見出しです", "readerImpact": "", "question": "",
            "evidence": [{ "blockId": 0, "quote": "申込み" }] })
            } else {
                json!({ "id": t["id"], "status": "unknown", "reason": "未確認",
            "readerImpact": "", "question": "", "evidence": [] })
            }
        })
        .collect();
    let response = json!({ "schemaVersion": 1, "requestHash": request["requestHash"],
        "reviewer": { "name": "fixture", "model": "local" }, "observations": observations,
        "coverage": [{ "id": "required_1", "status": "missing", "reason": "全文に期限がありません",
            "searched": "full_document", "evidence": [] }] });
    std::fs::write(&response_file, serde_json::to_vec(&response).unwrap()).unwrap();
    let output = cargo_bin_cmd!("noslop")
        .args([
            "review",
            file.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--response",
            response_file.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&output).unwrap();
    let span = &report["citations"][0]["span"];
    assert_eq!(
        &source[span["start"].as_u64().unwrap() as usize..span["end"].as_u64().unwrap() as usize],
        "申込み"
    );
    assert_eq!(report["review"]["coverage"][0]["status"], "missing");
    std::fs::write(&file, "# 案内\n\n受付は終わりました。\n").unwrap();
    cargo_bin_cmd!("noslop")
        .args([
            "review",
            file.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--response",
            response_file.to_str().unwrap(),
        ])
        .assert()
        .code(2);
}

#[test]
fn cli_review_rejects_fragments_and_conflicting_delivery_options() {
    cargo_bin_cmd!("noslop")
        .args(["review", "-", "--stdin-filename", "sample.rs"])
        .write_stdin("// 案内です。\n")
        .assert()
        .code(2);
    cargo_bin_cmd!("noslop")
        .args([
            "review",
            "-",
            "--response",
            "result.json",
            "--reviewer",
            "reviewer",
        ])
        .assert()
        .code(2);
}
