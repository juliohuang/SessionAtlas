import { test } from "node:test";
import assert from "node:assert/strict";
import { terminalScreenStatus } from "../core.js";
test("approval takes precedence over the cancel hint", () => {
  assert.equal(terminalScreenStatus("claude", "Do you want to proceed?\n > 1. Yes\n 2. No\nEsc to cancel ? Tab to amend"), "waiting");
});
test("silence, prose and plain shell prompts never imply completion", () => {
  for (const text of ["", "Task completed", "Do you want to proceed?"]) {
    assert.equal(terminalScreenStatus("claude", text), null);
  }
  assert.equal(terminalScreenStatus("claude", "Worked for 2m 3s\n\u276f "), "completed");
  assert.equal(terminalScreenStatus("codex", "Working (esc to interrupt)"), "running");
  assert.equal(terminalScreenStatus("custom", "Worked for 2m\n\u276f"), null);
});
test("returning to the input prompt clears a cancelled choice", () => {
  assert.equal(terminalScreenStatus("claude", "Do you want to proceed? Yes No\n❯ "), "ready");
});
test("previous turn completion cannot complete new work", () => {
  const previous = "Worked for 2m 3s\n❯ ";
  assert.equal(terminalScreenStatus("claude", previous + "\nNew request\nReading files..."), null);
  assert.equal(terminalScreenStatus("claude", previous, previous), null);
  assert.equal(terminalScreenStatus("claude", "Worked for 3m 4s\n❯ ", previous), "completed");
});
test("old approval outside the live tail is ignored", () => {
  assert.equal(terminalScreenStatus("claude", "Do you want to proceed? Yes No\n" + "new output\n".repeat(20)), null);
});
