import test from "node:test";
import assert from "node:assert/strict";
import { REVIEW_STRINGS, filterReviewSessions, gitStatusLabelKey } from "../project-review.js";

test("project review filters only local session title, preview, id, and selected tool", () => {
  const sessions = [
    { toolKey: "codex", title: "Release plan", preview: "Review the migration", sessionId: "cx-17" },
    { toolKey: "claude", title: "Bug triage", preview: "Release candidate", sessionId: "cl-22" },
  ];
  assert.deepEqual(filterReviewSessions(sessions, "migration").map(row => row.sessionId), ["cx-17"]);
  assert.deepEqual(filterReviewSessions(sessions, "CL-22").map(row => row.sessionId), ["cl-22"]);
  assert.deepEqual(filterReviewSessions(sessions, "release", "claude").map(row => row.sessionId), ["cl-22"]);
  assert.deepEqual(filterReviewSessions(sessions, "missing"), []);
});

test("English and Chinese review strings cover the same keys", () => {
  assert.deepEqual(Object.keys(REVIEW_STRINGS.en).sort(), Object.keys(REVIEW_STRINGS.zh).sort());
  assert.ok(Object.keys(REVIEW_STRINGS.en).every(key => key.startsWith("review.")));
  for (const reason of ["remote_session_unverified", "path_missing", "tool_missing", "tool_disabled", "unsupported_adapter", "unsupported_platform", "unsupported_resume", "session_missing", "history_incomplete", "ssh_unavailable", "check_failed", "launch_failed"]) {
    assert.ok(REVIEW_STRINGS.en[`review.resumeReason.${reason}`]);
    assert.ok(REVIEW_STRINGS.zh[`review.resumeReason.${reason}`]);
  }
});

test("porcelain XY statuses label staged and unstaged changes independently", () => {
  assert.equal(gitStatusLabelKey(" M", false), "review.statusModified");
  assert.equal(gitStatusLabelKey("M ", true), "review.statusModified");
  assert.equal(gitStatusLabelKey("R ", true), "review.statusRenamed");
  assert.equal(gitStatusLabelKey("??", false, true), "review.statusUntracked");
});
