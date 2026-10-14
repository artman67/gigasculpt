//! `boolean.apply` as a job (decision 0057, section 7): the sentences its run adds to the
//! warnings reach the job's answer as they reach a call that runs at once.

use serde_json::json;

use crate::support::Session;

#[test]
fn a_boolean_job_answers_with_its_warnings() {
    let s = Session::start("boolean-warnings");
    let mut a = s.client();
    a.run(
        "primitive.cube",
        json!({ "name": "body", "subdivisions": 2 }),
    );
    let over = json!({ "name": "cutter", "size": 3, "center": [1.6, 0, 0] });
    a.run("primitive.cube", over);
    let marks = [("body", "intersect"), ("cutter", "difference")];
    for (part, operation) in marks {
        let mark = json!({ "part": part, "operation": operation });
        a.run("part.set_operation", mark);
    }
    let report = a.start("boolean.apply", json!({}), 30_000);
    assert!(report["id"].is_u64(), "a long call is a job: {report}");
    let warnings = report["response"]["ok"]["warnings"].to_string();
    assert!(
        warnings.contains("'body'") && warnings.contains("intersect"),
        "{report}"
    );
}
