use super::*;

fn fixture(source: &str) -> (Request, Response) {
    let request = request(&Document::markdown(source), Profile::default()).unwrap();
    let response = Response {
        schema_version: SCHEMA_VERSION,
        request_hash: request.request_hash.clone(),
        reviewer: Reviewer {
            name: "fixture".into(),
            model: "local".into(),
        },
        coverage: vec![],
        observations: request
            .techniques
            .iter()
            .map(|t| Observation {
                id: t.id.into(),
                status: Assessment::Unknown,
                reason: "判断材料がありません".into(),
                reader_impact: String::new(),
                question: String::new(),
                evidence: vec![],
            })
            .collect(),
    };
    (request, response)
}

#[test]
fn request_covers_all_techniques_and_keeps_structure_separate() {
    let (request, response) = fixture("次の3項目です。\n\n- 氏名\n- 住所\n");
    assert_eq!(request.techniques.len(), 18);
    assert_eq!(request.structural_findings.len(), 1);
    assert_eq!(request.structural_findings[0].rule_id, "S11");
    assert!(!request.document.structure.nodes.is_empty());
    assert!(validate(&request, &response).is_ok());
}

#[test]
fn conditions_and_source_are_bound_to_the_result() {
    let (req, response) = fixture("申込みを受け付けます。");
    let other = request(
        &Document::markdown("申込みを受け付けます。"),
        Profile {
            purpose: Some("初回利用者が申し込む".into()),
            ..Profile::default()
        },
    )
    .unwrap();
    assert_eq!(req.document.source_hash, other.document.source_hash);
    assert_ne!(req.request_hash, other.request_hash);
    assert!(validate(&other, &response).is_err());
    let (changed, _) = fixture("申込みは受け付けません。");
    assert!(validate(&changed, &response).is_err());
}

#[test]
fn omitted_duplicate_or_invented_techniques_are_rejected() {
    let (req, mut response) = fixture("ご案内です。");
    let last = response.observations.pop().unwrap();
    assert!(validate(&req, &response).is_err());
    response.observations.push(last);
    response.observations[1].id = response.observations[6].id.clone();
    assert!(validate(&req, &response).is_err());
    response.observations[1].id = "invented".into();
    assert!(validate(&req, &response).is_err());
}

#[test]
fn citations_resolve_exact_japanese_source_and_disambiguate_repetition() {
    let source = "# 案内\n\n**申込み**を受け付けます。申込みには住所が必要です。\n";
    let (req, mut response) = fixture(source);
    let item = &mut response.observations[6];
    item.status = Assessment::NeedsReview;
    item.reader_impact = "期限が分かりません".into();
    item.question = "いつまで受け付けますか".into();
    item.evidence.push(Evidence {
        block_id: 1,
        quote: "申込み".into(),
        occurrence: None,
    });
    assert!(validate(&req, &response).is_err());
    response.observations[6].evidence[0].occurrence = Some(2);
    let citations = validate(&req, &response).unwrap();
    assert_eq!(&source[citations[0].span.range()], "申込み");
    assert_eq!(citations[0].span.start, source.rfind("申込み").unwrap());
    response.observations[6].evidence[0].quote = "受付は明日まで".into();
    assert!(validate(&req, &response).is_err());
}

#[test]
fn claims_need_evidence_and_reader_effect_and_questions() {
    let (req, mut response) = fixture("申込みを受け付けます。");
    response.observations[6].status = Assessment::Clear;
    assert!(validate(&req, &response).is_err());
    response.observations[6].evidence.push(Evidence {
        block_id: 0,
        quote: "申込み".into(),
        occurrence: None,
    });
    assert!(validate(&req, &response).is_ok());
    response.observations[6].status = Assessment::NeedsReview;
    assert!(validate(&req, &response).is_err());
    response.observations[6].reader_impact = "確認できません".into();
    response.observations[6].question = "必要な情報は何ですか".into();
    assert!(validate(&req, &response).is_ok());
    response.observations[6].evidence[0].block_id = 100;
    assert!(validate(&req, &response).is_err());
}

#[test]
fn profile_typos_and_extra_response_fields_are_errors() {
    assert!(serde_json::from_str::<Profile>(r#"{"purpos":"案内"}"#).is_err());
    let (_, response) = fixture("案内です。");
    let mut json = serde_json::to_value(&response).unwrap();
    json["observations"][0]["replacement"] = "勝手に改稿".into();
    assert!(serde_json::from_value::<Response>(json).is_err());
}

#[test]
fn independent_code_fragments_cannot_be_reviewed_as_an_article() {
    let doc = Document::parse(
        "example.rs",
        "// 案内です。",
        SourceFormat::from_path(std::path::Path::new("example.rs")),
        &ParseOptions::default(),
    );
    assert!(request(&doc, Profile::default()).is_err());
}

#[test]
fn prerequisites_and_declared_information_are_checked_separately() {
    let (unscoped, mut response) = fixture("申込みを受け付けます。");
    response.observations[0].status = Assessment::Clear;
    response.observations[0].evidence.push(Evidence {
        block_id: 0,
        quote: "申込み".into(),
        occurrence: None,
    });
    assert!(validate(&unscoped, &response).unwrap_err().contains("前提"));
    let scoped = request(
        &Document::markdown("申込みを受け付けます。"),
        Profile {
            purpose: Some("申込みの案内".into()),
            audience: Some("初めて申し込む人".into()),
            required_information: vec!["受付期限".into()],
            ..Profile::default()
        },
    )
    .unwrap();
    response.request_hash = scoped.request_hash.clone();
    response.coverage.push(InformationCoverage {
        id: "required_1".into(),
        status: CoverageStatus::Missing,
        reason: "全文に受付期限がありません".into(),
        searched: None,
        evidence: vec![],
    });
    assert!(validate(&scoped, &response).unwrap_err().contains("全文"));
    response.coverage[0].searched = Some(SearchScope::FullDocument);
    assert!(validate(&scoped, &response).unwrap_err().contains("clear"));
    response.observations[0].status = Assessment::NeedsReview;
    response.observations[0].evidence.clear();
    response.observations[0].reader_impact = "締切を判断できません".into();
    response.observations[0].question = "期限を案内に含めますか".into();
    assert!(validate(&scoped, &response).is_ok());
    response.coverage[0].status = CoverageStatus::Present;
    assert!(validate(&scoped, &response).is_err());
    response.coverage.clear();
    assert!(
        validate(&scoped, &response)
            .unwrap_err()
            .contains("必須情報")
    );
}

#[test]
fn display_name_is_private_and_not_a_review_condition_and_suppression_is_preserved() {
    let doc = Document::parse(
        "private/path/guide.md",
        "# 案内\n\n本文です。",
        SourceFormat::Markdown,
        &ParseOptions::default(),
    );
    let first = request(&doc, Profile::default()).unwrap();
    assert_eq!(first.document.name, "guide.md");
    let mut doc2 = doc.clone();
    doc2.name = "./guide.md".into();
    assert_eq!(
        first.request_hash,
        request(&doc2, Profile::default()).unwrap().request_hash
    );
    assert!(
        first
            .document
            .structure
            .nodes
            .iter()
            .any(|n| n.block_ids == vec![1])
    );
    let suppressed = Document::markdown("<!-- noslop-disable-file S12 -- 下書き -->\n\n# 案内\n");
    assert!(
        request(&suppressed, Profile::default())
            .unwrap()
            .structural_findings
            .is_empty()
    );
    let blank = request(
        &doc,
        Profile {
            purpose: Some("  ".into()),
            ..Profile::default()
        },
    )
    .unwrap();
    assert!(blank.profile.purpose.is_none());
}

#[cfg(unix)]
#[test]
fn timeout_kills_the_reviewers_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("descendant-output");
    let result = runner::invoke(
        std::path::Path::new("/bin/sh"),
        &[
            "-c".into(),
            "(sleep 2; printf survived > \"$1\") & wait".into(),
            "review".into(),
            marker.display().to_string(),
        ],
        vec![],
        1,
    );
    assert!(result.is_err());
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(!marker.exists(), "子孫をタイムアウトで終了する");
}

#[cfg(unix)]
#[test]
fn reviewer_receives_stdin_and_arguments_without_shell_expansion() {
    // 待ち時間はこのテストの対象ではない。CI の VM が数十秒止まっても期限切れで落ちないよう、
    // CLI の既定 (120 秒) と同じにする
    let output = runner::invoke(
        std::path::Path::new("/bin/sh"),
        &[
            "-c".into(),
            "cat; printf '%s' \"$1\"".into(),
            "review".into(),
            "$(exit 9)".into(),
        ],
        "本文".as_bytes().to_vec(),
        120,
    )
    .unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "本文$(exit 9)");
}

#[cfg(unix)]
#[test]
fn failed_or_stalled_reviewer_does_not_produce_a_report() {
    let sh = std::path::Path::new("/bin/sh");
    assert!(runner::invoke(sh, &["-c".into(), "exit 7".into()], vec![], 5).is_err());
    assert!(runner::invoke(sh, &["-c".into(), "exec sleep 3".into()], vec![], 1).is_err());
}
