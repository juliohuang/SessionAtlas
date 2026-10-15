import { expect, test } from "@playwright/test";

async function installRemoteTerminalFixture(page) {
  await page.addInitScript(() => {
    const usage = (toolKey, lastSessionId) => ({
      toolKey,
      toolName: toolKey === "codex" ? "Codex CLI" : "Claude Code",
      lastUsedAt: "2026-08-16T00:00:00Z",
      sessionCount: 1,
      lastSessionId,
    });
    window.__remoteProjects = [
      {
        id: "remote-one",
        path: "/srv/project-one",
        name: "project-one",
        lastAccessedAt: "2026-08-16T00:00:00Z",
        gitBranch: "main",
        remoteServerId: 7,
        toolUsages: [usage("codex", "codex-session")],
      },
      {
        id: "remote-two",
        path: "/srv/project-two",
        name: "project-two",
        lastAccessedAt: "2026-08-15T00:00:00Z",
        gitBranch: "feature/two",
        remoteServerId: 7,
        toolUsages: [usage("claude", "claude-session")],
      },
    ];
    window.__invokeCalls = [];
    window.__rejectRemoteSwitch = false;
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (command === "list_projects" || command === "search_projects") return [];
          if (command === "list_remote_projects" || command === "search_remote_projects") {
            return window.__remoteProjects;
          }
          if (command === "list_remote_servers") {
            return [{
              id: 7,
              label: "development-box",
              user: "tester",
              host: "192.0.2.7",
              port: 22,
              identityFile: null,
            }];
          }
          if (command === "list_tools") {
            return [
              { toolKey: "codex", toolName: "Codex CLI" },
              { toolKey: "claude", toolName: "Claude Code" },
            ];
          }
          if (command === "probe_tui_capabilities") {
            return {
              source: payload?.serverId == null ? "local" : "remote",
              serverId: payload?.serverId ?? null,
              label: payload?.serverId == null ? "Local" : "development-box",
              tools: [
                { toolKey: "codex", toolName: "Codex CLI", installed: true, version: "test", enabled: true, installAvailable: true, installManager: "npm" },
                { toolKey: "claude", toolName: "Claude Code", installed: true, version: "test", enabled: true, installAvailable: true, installManager: "npm" },
              ],
            };
          }
          if (command === "pty_spawn") return 101;
          if (command === "pty_remote_switch" && window.__rejectRemoteSwitch) {
            throw new Error("switch rejected");
          }
          if (command === "list_groups" || command === "list_sort_orders"
              || command === "list_opener_prefs") return [];
          if (command === "list_group_assignments") return {};
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          return null;
        },
      },
      event: { listen: async (name, callback) => { (window.__ptyListeners ||= {})[name] = callback; return () => {}; } },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });
}

test("same remote server reuses one SSH PTY and switches its tmux target", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="remote-one"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="codex"]').click();
  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-one");

  await page.locator('article.entry[data-id="remote-two"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();

  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-two");
  await expect(page.locator("#footStatus")).toContainText("development-box");

  const calls = await page.evaluate(() => window.__invokeCalls);
  expect(calls.filter(call => call.command === "pty_spawn")).toHaveLength(1);
  expect(calls.filter(call => call.command === "pty_attach")).toHaveLength(1);
  const switches = calls.filter(call => call.command === "pty_remote_switch");
  expect(switches).toEqual([{
    command: "pty_remote_switch",
    payload: {
      id: 101,
      path: "/srv/project-two",
      serverId: 7,
      toolKey: "claude",
      sessionId: "claude-session",
    },
  }]);
});

test("failed remote tmux switch keeps the original tab target", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="remote-one"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="codex"]').click();
  await expect(page.locator(".term-tab__name")).toHaveText("project-one");

  await page.evaluate(() => { window.__rejectRemoteSwitch = true; });
  await page.locator('article.entry[data-id="remote-two"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();

  await expect(page.locator(".term-tab")).toHaveCount(1);
  await expect(page.locator(".term-tab__name")).toHaveText("project-one");
  await expect(page.locator("#footStatus")).toContainText("switch rejected");
});

test("IME composition cannot horizontally shift the terminal workspace", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");

  await page.locator('article.entry[data-id="remote-one"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="codex"]').click();
  await expect(page.locator(".term-pane.is-active .xterm-helper-textarea")).toHaveCount(1);

  const workspaceBefore = await page.locator(".stage__right").boundingBox();
  const result = await page.evaluate(async () => {
    const pane = document.querySelector(".term-pane.is-active");
    const textarea = pane.querySelector(".xterm-helper-textarea");
    const viewport = document.getElementById("termsViewport");
    const overflowProbe = document.createElement("div");
    overflowProbe.style.cssText = "position:absolute;left:0;top:0;width:4000px;height:1px";
    pane.appendChild(overflowProbe);

    textarea.dispatchEvent(new Event("compositionstart"));
    const composingDuringInput = pane.classList.contains("is-composing");
    pane.scrollLeft = 320;
    viewport.scrollLeft = 240;
    textarea.dispatchEvent(new Event("compositionupdate"));
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const offsetsDuringInput = {
      pane: pane.scrollLeft,
      viewport: viewport.scrollLeft,
      document: document.scrollingElement.scrollLeft,
    };

    const resizeCountBeforeEnd = window.__invokeCalls
      .filter(call => call.command === "pty_resize").length;
    textarea.dispatchEvent(new Event("compositionend"));
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const resizeCountAfterEnd = window.__invokeCalls
      .filter(call => call.command === "pty_resize").length;
    overflowProbe.remove();

    return {
      composingDuringInput,
      composingAfterInput: pane.classList.contains("is-composing"),
      offsetsDuringInput,
      resizeCountBeforeEnd,
      resizeCountAfterEnd,
    };
  });
  const workspaceAfter = await page.locator(".stage__right").boundingBox();

  expect(result.composingDuringInput).toBe(true);
  expect(result.composingAfterInput).toBe(false);
  expect(result.offsetsDuringInput).toEqual({ pane: 0, viewport: 0, document: 0 });
  expect(result.resizeCountAfterEnd).toBeGreaterThan(result.resizeCountBeforeEnd);
  expect(workspaceAfter.x).toBe(workspaceBefore.x);
  expect(workspaceAfter.width).toBe(workspaceBefore.width);
});

test("terminal and project show waiting, running, completion and failure", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");
  await page.locator('article.entry[data-id="remote-two"]').click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();
  const badge = page.locator('.term-tab [data-terminal-status]');
  await expect(badge).toHaveAttribute("data-terminal-status", "ready");
  const output = async (data) => page.evaluate(data => window.__ptyListeners["pty-data"]({payload:{id:101,data}}), data);
  await output("Do you want to pro");
  await output("ceed?\r\n> 1. Yes\r\n 2. No\r\nEsc to cancel ? Tab to amend");
  await expect(badge).toHaveAttribute("data-terminal-status", "waiting");
  await expect(page.locator('article[data-id="remote-two"] [data-terminal-status]')).toHaveAttribute("data-terminal-status", "waiting");
  await output("\x1b[2J\x1b[HWorking (esc to interrupt)");
  await expect(badge).toHaveAttribute("data-terminal-status", "running");
  await output("\x1b[2J\x1b[HWorked for 2m 3s\r\n\u276f ");
  await expect(badge).toHaveAttribute("data-terminal-status", "completed");
  await page.evaluate(() => window.__ptyListeners["pty-exit"]({payload:{id:101,exitCode:1,readError:null}}));
  await expect(badge).toHaveAttribute("data-terminal-status", "failed");
  await output("\x1b[2J\x1b[HWorked for 1m\r\n\u276f ");
  await expect(badge).toHaveAttribute("data-terminal-status", "failed");
});

test("status updates preserve unfocused drafts and reject stale completion", async ({ page }) => {
  await installRemoteTerminalFixture(page);
  await page.goto("/index.html");
  const entry = page.locator('article.entry[data-id="remote-two"]');
  await entry.click();
  await page.locator('#termsSelectedLaunch [data-tool="claude"]').click();
  await entry.locator('[data-expand-toggle]').click();
  const draft = entry.locator('[data-queue-input]');
  await draft.fill("Keep my unsubmitted task");
  const input = page.locator('.term-pane.is-active .xterm-helper-textarea');
  await input.focus();
  const badge = page.locator('.term-tab [data-terminal-status]');
  const output = async data => page.evaluate(data => window.__ptyListeners['pty-data']({payload:{id:101,data}}), data);
  await output("Do you want to proceed?\r\n1. Yes\r\n2. No\r\nEsc to cancel");
  await expect(badge).toHaveAttribute('data-terminal-status', 'waiting');
  await expect(draft).toHaveValue("Keep my unsubmitted task");
  await input.press('Escape');
  await output("\x1b[2J\x1b[H❯ ");
  await expect(badge).toHaveAttribute('data-terminal-status', 'ready');
  await output("\x1b[2J\x1b[HWorked for 2m 3s\r\n❯ ");
  await expect(badge).toHaveAttribute('data-terminal-status', 'completed');
  await input.press('Enter');
  await expect(badge).toHaveAttribute('data-terminal-status', 'running');
  await output("\r\nNew request\r\nReading files...");
  await expect(badge).toHaveAttribute('data-terminal-status', 'running');
  // A redraw of the old completed screen is not a new completion event.
  await output("\x1b[2J\x1b[HWorked for 2m 3s\r\n❯ ");
  await expect(badge).toHaveAttribute('data-terminal-status', 'running');
  await output("\x1b[2J\x1b[HWorked for 3m 4s\r\n❯ ");
  await expect(badge).toHaveAttribute('data-terminal-status', 'completed');
  await page.evaluate(() => window.__ptyListeners['pty-exit']({payload:{id:101,exitCode:1}}));
  await expect(badge).toHaveAttribute('data-terminal-status', 'failed');
  await expect(draft).toHaveValue("Keep my unsubmitted task");
  await expect(input).toBeFocused();
});
