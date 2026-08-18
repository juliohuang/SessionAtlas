import { expect, test } from "@playwright/test";

async function installMutationFixture(page) {
  await page.addInitScript(() => {
    const project = {
      id: "p1",
      path: "C:\\workspace\\p1",
      name: "project-one",
      osFamily: "windows",
      lastAccessedAt: "2026-08-03T00:00:00Z",
      gitBranch: "main",
      toolUsages: [],
    };
    window.__rejectCommands = [];
    window.__invokeCalls = [];
    window.__remoteServers = [];
    window.__projectIgnores = [];
    window.__docText = [
      "# Fixture document",
      "",
      "[![CI](https://images.example.test/ci.svg)](https://example.test/ci)",
      "![Local preview](./preview.png)",
      "[Relative guide](./GUIDE.md)",
      "[Unsafe](javascript:alert(1))",
    ].join("\n");
    window.__holdRemoteScan = false;
    window.__resolveRemoteScan = null;
    window.__holdRemoteProbe = false;
    window.__resolveRemoteProbe = null;
    window.__tmuxProbe = {
      home: "/home/tester",
      osFamily: "linux",
      tmuxAvailable: true,
      tmuxVersion: "tmux 3.4",
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          window.__invokeCalls.push({ command, payload });
          if (window.__rejectCommands.includes(command)) throw new Error(`${command} denied`);
          if (command === "list_projects" || command === "search_projects") {
            return window.__projectIgnores.length ? [] : [project];
          }
          if (command === "list_tools" || command === "list_remote_projects"
              || command === "search_remote_projects") return [];
          if (command === "list_remote_servers") return window.__remoteServers;
          if (command === "probe_tui_capabilities") {
            const serverId = payload?.serverId ?? null;
            const server = window.__remoteServers.find(item => item.id === serverId);
            return {
              source: serverId == null ? "local" : "remote",
              serverId,
              label: server?.label || (serverId == null ? "Local" : "Remote"),
              tools: [],
              adapterDiagnostics: [],
            };
          }
          if (command === "list_project_ignores") return window.__projectIgnores;
          if (command === "list_project_docs") {
            return [{ name: "README.md", relPath: "README.md", size: window.__docText.length }];
          }
          if (command === "read_project_doc" || command === "read_text_file") {
            return window.__docText;
          }
          if (command === "list_dir") {
            return [{
              name: "README.md",
              path: "C:\\workspace\\p1\\README.md",
              isDir: false,
              size: window.__docText.length,
            }];
          }
          if (command === "add_project_ignore") {
            const rule = {
              id: 21,
              source: payload.source,
              remoteServerId: payload.remoteServerId,
              path: payload.path,
              createdAt: "2026-08-17T00:00:00Z",
            };
            window.__projectIgnores = [rule];
            return rule;
          }
          if (command === "delete_project_ignore") {
            window.__projectIgnores = window.__projectIgnores
              .filter(rule => rule.id !== payload.ignoreId);
            return null;
          }
          if (command === "test_remote_connection") {
            if (window.__holdRemoteProbe) {
              return new Promise(resolve => {
                window.__resolveRemoteProbe = () => resolve(window.__tmuxProbe);
              });
            }
            return window.__tmuxProbe;
          }
          if (command === "scan_remote_server") {
            const completeScan = count => {
              const scannedAt = new Date().toISOString();
              window.__remoteServers = window.__remoteServers.map(server => (
                server.id === payload.serverId ? { ...server, lastScannedAt: scannedAt } : server
              ));
              return count;
            };
            if (window.__holdRemoteScan) {
              return new Promise(resolve => {
                window.__resolveRemoteScan = count => resolve(completeScan(count));
              });
            }
            return completeScan(0);
          }
          if (command === "add_remote_server") {
            const server = {
              id: 11,
              label: payload.label,
              user: payload.user,
              host: payload.host,
              port: payload.port || 22,
              identityFile: payload.identityFile,
              scanRoots: [],
              lastScannedAt: null,
              osFamily: payload.osFamily,
            };
            window.__remoteServers = [server];
            return server;
          }
          if (command === "rename_remote_server") {
            window.__remoteServers = window.__remoteServers.map(server => (
              server.id === payload.serverId ? { ...server, label: payload.label.trim() } : server
            ));
            return null;
          }
          if (command === "list_opener_prefs") {
            return [{
              id: 7,
              type: "custom",
              builtinKey: null,
              label: "Editor",
              command: "editor {path}",
              enabled: true,
              sortOrder: 10,
            }];
          }
          if (command === "list_groups") {
            return [{ id: 3, name: "Original", sortOrder: 10, memberCount: 1 }];
          }
          if (command === "list_group_assignments") return { p1: 3 };
          if (command === "list_sort_orders") return [];
          if (command === "get_group_revision") return 0;
          if (command === "get_git_info") return { isRepo: false, remotes: [] };
          return null;
        },
      },
      event: { listen: async () => () => {} },
      window: { getCurrentWindow: () => ({ isMaximized: async () => false }) },
    };
  });
}

async function openSettingsView(page, view) {
  await page.locator("#settingsBtn").click();
  await page.locator(`[data-settings-view="${view}"]`).click();
}

test("typing in a settings form does not trigger the Escape-only drawer handler", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");

  const label = page.locator('#serverForm [name="label"]');
  await expect(page.locator(".ssh-command-field__prefix")).toHaveText("ssh");
  await label.pressSequentially("E2E remote server");

  await expect(label).toHaveValue("E2E remote server");
  await expect(page.locator("#drawerTitle")).toHaveText("Remote servers");
  await expect(page.locator("#drawer")).toBeVisible();

  await label.press("Escape");
  await expect(page.locator('[data-settings-view="remote"]')).toBeVisible();
  await expect(page.locator("#drawer")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#drawer")).toBeHidden();
});

test("a project directory tree can be ignored and restored from settings", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");

  const project = page.locator('article.entry[data-id="p1"]');
  await expect(project).toBeVisible();
  await project.click();
  await page.locator("#termsSelectedLaunch [data-ignore-project]").click();

  await expect(project).toHaveCount(0);
  await expect(page.locator("#footStatus")).toContainText("ignored this directory tree");
  const addCall = await page.evaluate(() => window.__invokeCalls
    .findLast(call => call.command === "add_project_ignore"));
  expect(addCall.payload).toEqual({
    source: "local",
    remoteServerId: null,
    path: "C:\\workspace\\p1",
  });

  await openSettingsView(page, "ignores");
  await expect(page.locator(".ignore-rule-note")).toContainText("starts with .");
  await expect(page.locator('.ignore-row[data-id="21"] .ignore-row__path'))
    .toHaveText("C:\\workspace\\p1");
  await page.locator('.ignore-row[data-id="21"] [data-ignore-del]').click();

  await expect(page.locator(".ignore-row")).toHaveCount(0);
  await expect(project).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("visible again");
});

test("failed opener toggle restores the checkbox without replacing the ledger", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
  await openSettingsView(page, "openers");
  await page.evaluate(() => window.__rejectCommands.push("set_opener_enabled"));

  const checkbox = page.locator('[data-opener-toggle]');
  await expect(checkbox).toBeChecked();
  await checkbox.click();

  await expect(checkbox).toBeChecked();
  await expect(page.locator("#footStatus")).toContainText("set_opener_enabled denied");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
});

test("failed opener create and delete preserve the draft and existing row", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "openers");
  await page.evaluate(() => window.__rejectCommands.push("upsert_custom_opener", "delete_custom_opener"));

  await page.locator('#customForm [name="label"]').fill("Draft opener");
  await page.locator('#customForm [name="command"]').fill("draft {path}");
  await page.locator('#customForm [type="submit"]').click();

  await expect(page.locator('#customForm [name="label"]')).toHaveValue("Draft opener");
  await expect(page.locator('#customForm [name="command"]')).toHaveValue("draft {path}");
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();

  await page.locator('.opener-row[data-id="7"] [data-opener-del]').click();
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("delete_custom_opener denied");
});

test("failed group rename and create preserve canonical name and form draft", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "groups");
  await page.evaluate(() => window.__rejectCommands.push("rename_group", "create_group"));

  const rename = page.locator('.group-row[data-id="3"] [data-group-rename]');
  await rename.fill("Unsaved rename");
  await expect(rename).toHaveClass(/is-unsaved/);
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();

  await page.locator('#groupForm [name="name"]').fill("Draft group");
  await page.locator('#groupForm [type="submit"]').click();
  await expect(page.locator('#groupForm [name="name"]')).toHaveValue("Draft group");
  await expect(page.locator('.group-row[data-id="3"]')).toBeVisible();
  await expect(page.locator("#footStatus")).toContainText("create_group denied");
});

test("server add followed by scan failure is explicit partial success", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => window.__rejectCommands.push("scan_remote_server"));

  await page.locator('#serverForm [name="label"]').fill("Remote A");
  await page.locator('#serverForm [name="sshCommand"]').fill("ssh -p 2222 tester@example.test");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator("#footStatus")).toContainText("was added, but its initial scan failed");
  await expect(page.locator('.server-row[data-id="11"]')).toBeVisible();
  await expect(page.locator('.server-row[data-id="11"] [data-server-last-scan]'))
    .toHaveText("Not scanned yet");
  await expect(page.locator('#serverForm [name="label"]')).toHaveValue("");
  await expect(page.locator('article.entry[data-id="p1"]')).toBeVisible();
  const connectionCalls = await page.evaluate(() => window.__invokeCalls
    .filter(call => call.command === "test_remote_connection" || call.command === "add_remote_server"));
  expect(connectionCalls[0].payload).toEqual({
    user: "tester",
    host: "example.test",
    port: 2222,
    identityFile: null,
  });
  expect(connectionCalls[1].payload).toMatchObject({
    label: "Remote A",
    user: "tester",
    host: "example.test",
    port: 2222,
    identityFile: null,
    osFamily: "linux",
  });
  await expect(page.locator('.server-row[data-id="11"] .machine-identity'))
    .toHaveAttribute("data-machine-kind", "remote");
  await expect(page.locator('.server-row[data-id="11"] .machine-identity'))
    .toHaveAttribute("data-os-family", "linux");
});

test("server add warns when the remote machine has no tmux", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => {
    window.__tmuxProbe = {
      home: "/home/tester",
      tmuxAvailable: false,
      tmuxVersion: null,
    };
  });

  await page.locator('#serverForm [name="sshCommand"]').fill("tester@example.test");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator("#footStatus")).toContainText("tmux is not installed");
  await expect(page.locator("#footStatus")).toContainText("sudo apt install tmux");
  await expect(page.locator('.server-row[data-id="11"]')).toBeVisible();
  await expect(page.locator('.server-row[data-id="11"] .server-row__label')).toHaveValue("example.test");
});

test("SSH address, machine name, and add button stay on one row", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");

  const [sshBox, nameBox, buttonBox] = await Promise.all([
    page.locator("#serverForm .ssh-command-field").boundingBox(),
    page.locator('#serverForm [name="label"]').boundingBox(),
    page.locator('#serverForm [type="submit"]').boundingBox(),
  ]);
  expect(sshBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(buttonBox).not.toBeNull();
  const sshCenterY = sshBox.y + sshBox.height / 2;
  const nameCenterY = nameBox.y + nameBox.height / 2;
  const buttonCenterY = buttonBox.y + buttonBox.height / 2;
  expect(Math.abs(sshCenterY - nameCenterY)).toBeLessThan(2);
  expect(Math.abs(sshCenterY - buttonCenterY)).toBeLessThan(2);
  expect(nameBox.x).toBeGreaterThan(sshBox.x + sshBox.width);
  expect(buttonBox.x).toBeGreaterThan(nameBox.x + nameBox.width);
});

test("SSH host:port input gives immediate progress and separates the port", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => { window.__holdRemoteProbe = true; });

  await page.locator('#serverForm [name="sshCommand"]').fill("root@101.133.150.255:9336");
  await page.locator('#serverForm [name="label"]').fill("nuc");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator('#serverForm [type="submit"]')).toBeDisabled();
  await expect(page.locator('#serverForm [type="submit"]')).toHaveText("CONNECTING…");
  await expect(page.locator("[data-server-form-feedback]")).toContainText("testing passwordless connection");
  await expect.poll(() => page.evaluate(() => window.__invokeCalls
    .findLast(call => call.command === "test_remote_connection")?.payload)).toEqual({
    user: "root",
    host: "101.133.150.255",
    port: 9336,
    identityFile: null,
  });

  await page.evaluate(() => window.__resolveRemoteProbe());
  await expect(page.locator('.server-row[data-id="11"]')).toBeVisible();
  await expect(page.locator('.server-row[data-id="11"] .server-row__conn'))
    .toHaveText("root@101.133.150.255:9336");
});

test("server add returns while scanning and the display name remains editable", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");
  await page.evaluate(() => { window.__holdRemoteScan = true; });

  await page.locator('#serverForm [name="sshCommand"]').fill("tester@example.test");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator('#serverForm [name="sshCommand"]')).toHaveValue("");
  await expect(page.locator('#serverForm [type="submit"]')).toBeEnabled();
  await expect(page.locator('.server-row[data-id="11"] [data-server-scan]')).toBeDisabled();
  await expect(page.locator('.server-row[data-id="11"] [data-server-last-scan]'))
    .toHaveText("Not scanned yet");
  await expect(page.locator("#footStatus")).toContainText("initial scan is running in the background");
  await expect.poll(() => page.evaluate(() => window.__invokeCalls.some(call => (
    call.command === "probe_tui_capabilities" && call.payload?.serverId === 11
  )))).toBe(true);

  const name = page.locator('.server-row[data-id="11"] [data-server-rename]');
  await name.fill("Build machine");
  await expect.poll(() => page.evaluate(() => window.__invokeCalls
    .findLast(call => call.command === "rename_remote_server")?.payload?.label)).toBe("Build machine");

  await page.evaluate(() => window.__resolveRemoteScan(4));
  await expect(page.locator('.server-row[data-id="11"] [data-server-scan]')).toBeEnabled();
  await expect(name).toHaveValue("Build machine");
  await expect(page.locator('.server-row[data-id="11"] [data-server-last-scan]'))
    .toHaveText("Last scan · just now");
  await expect(page.locator('.server-row[data-id="11"] [data-server-last-scan]'))
    .toHaveAttribute("title", /Last successful scan:/);
  await expect(page.locator("#footStatus")).toContainText("Build machine: 4 projects");
});

test("SSH form rejects remote command text before invoking the backend", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "remote");

  await page.locator('#serverForm [name="sshCommand"]').fill("tester@example.test whoami");
  await page.locator('#serverForm [type="submit"]').click();

  await expect(page.locator("#footStatus")).toContainText("remote commands are not supported");
  await expect(page.locator("[data-server-form-feedback]")).toContainText("remote commands are not supported");
  await expect(page.locator(".ssh-command-field")).toHaveClass(/is-invalid/);
  const probeCount = await page.evaluate(() => window.__invokeCalls
    .filter(call => call.command === "test_remote_connection").length);
  expect(probeCount).toBe(0);
});

test("successful write followed by reconciliation failure keeps last-known-good rows", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "groups");
  await page.evaluate(() => window.__rejectCommands.push("list_groups"));

  await page.locator('#groupForm [name="name"]').fill("Committed group");
  await page.locator('#groupForm [type="submit"]').click();
  await expect(page.locator('.group-row[data-id="3"]')).toBeVisible();
  await expect(page.locator('#groupForm [name="name"]')).toHaveValue("");
  await expect(page.locator("#footStatus")).toContainText("stale: groups");

  await page.locator("#drawerBackBtn").click();
  await page.locator('[data-settings-view="openers"]').click();
  await page.evaluate(() => {
    window.__rejectCommands = window.__rejectCommands.filter(command => command !== "list_groups");
    window.__rejectCommands.push("list_opener_prefs");
  });
  await page.locator('#customForm [name="label"]').fill("Committed opener");
  await page.locator('#customForm [name="command"]').fill("cmd {path}");
  await page.locator('#customForm [type="submit"]').click();
  await expect(page.locator('.opener-row[data-id="7"]')).toBeVisible();
  await expect(page.locator('#customForm [name="label"]')).toHaveValue("");
});

test("dialogs isolate the workspace, trap focus, and return focus to their opener", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");

  const settingsButton = page.locator("#settingsBtn");
  await settingsButton.click();
  await expect(page.locator(".console")).toHaveAttribute("inert", "");
  await expect(page.locator('[data-settings-view="language"]')).toBeFocused();

  await page.locator('[data-settings-view="remote"]').click();
  await expect(page.locator("#drawerBackBtn")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings-view="language"]')).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator("#drawer")).toBeHidden();
  await expect(page.locator(".console")).not.toHaveAttribute("inert", "");
  await expect(settingsButton).toBeFocused();

  const projectMenuButton = page.locator('article.entry[data-id="p1"] [data-menu-toggle]');
  await projectMenuButton.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("button[data-entry-modal-close]")).toBeFocused();
  await expect(page.locator(".console")).toHaveAttribute("inert", "");
  expect(await page.evaluate(() => window.__invokeCalls
    .filter(call => call.command === "pty_spawn").length)).toBe(0);
  await expect(page.locator(".doc-pill")).toHaveCount(1);

  await page.locator(".doc-pill").click();
  await expect(page.locator("button[data-doc-modal-close]")).toBeFocused();
  await page.locator("button[data-doc-modal-close]").click();
  await expect(page.locator("#docModal")).toBeHidden();
  await expect(page.locator(".console")).not.toHaveAttribute("inert", "");
  await expect(projectMenuButton).toBeFocused();
});

test("markdown badges and images render as safe labelled chips without network fetches", async ({ page }) => {
  const externalRequests = [];
  page.on("request", request => {
    const url = new URL(request.url());
    if (url.hostname !== "127.0.0.1") externalRequests.push(request.url());
  });
  await installMutationFixture(page);
  await page.goto("/index.html");
  await page.locator('article.entry[data-id="p1"] [data-menu-toggle]').dispatchEvent("click");
  await page.locator(".doc-pill").click();

  const body = page.locator("#docModalBody");
  await expect(body.locator(".md-image-link")).toHaveText("▧CI");
  await expect(body.locator(".md-image-link"))
    .toHaveAttribute("href", "https://example.test/ci");
  await expect(body.locator(".md-image-placeholder")).toContainText("Local preview");
  await expect(body.locator(".md-link-muted")).toHaveText("Relative guide");
  await expect(body).not.toContainText("![");
  await expect(body.locator('a[href^="javascript:"]')).toHaveCount(0);
  expect(externalRequests).toEqual([]);
});

test("the file workspace uses the same clean markdown rendering", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");

  await page.locator("#termsSelectedLaunch [data-overview-files]").click();
  const readme = page.locator('.stage__left__files .tree__node--file .tree__name');
  await expect(readme).toHaveText("README.md");
  await readme.click();

  const fileBody = page.locator(".file-pane__body");
  await expect(fileBody.locator(".md-image-link")).toHaveText("▧CI");
  await expect(fileBody.locator(".md-image-placeholder")).toContainText("Local preview");
  await expect(fileBody).not.toContainText("![");
  await expect(page.locator(".file-pane")).toBeVisible();
  await expect(page.locator(".terms__other-tab")).toContainText("README.md");
});

test("all primary surfaces fit the supported minimum window", async ({ page }) => {
  await page.setViewportSize({ width: 980, height: 640 });
  await installMutationFixture(page);
  await page.goto("/index.html");

  const expectInsideViewport = async selector => {
    const box = await page.locator(selector).boundingBox();
    expect(box).not.toBeNull();
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(980);
    expect(box.y + box.height).toBeLessThanOrEqual(640);
  };

  await expectInsideViewport(".deck");
  await expectInsideViewport(".stage");
  await expectInsideViewport(".foot");

  await page.locator("#settingsBtn").click();
  await page.waitForTimeout(250);
  await expectInsideViewport(".drawer__panel");
  for (const view of ["remote", "ignores", "groups", "openers", "language"]) {
    await page.locator(`[data-settings-view="${view}"]`).click();
    await expectInsideViewport(".drawer__panel");
    await expect(page.locator("#drawerTitle")).not.toBeEmpty();
    await page.locator("#drawerBackBtn").click();
  }
  await page.keyboard.press("Escape");

  await page.locator('article.entry[data-id="p1"] [data-menu-toggle]').dispatchEvent("click");
  await expectInsideViewport(".entry-modal__panel");
  await page.locator(".doc-pill").click();
  await expectInsideViewport(".doc-modal__panel");
});

test("language settings update every visible surface and can be restored", async ({ page }) => {
  await installMutationFixture(page);
  await page.goto("/index.html");
  await openSettingsView(page, "language");

  await expect(page.locator(".lang-picker input")).toHaveCount(0);
  await expect(page.locator('.lang-row[data-lang-select="en"]')).toHaveClass(/is-active/);
  await expect(page.locator('.lang-row[data-lang-select="en"]')).toHaveAttribute("aria-checked", "true");

  await page.locator('.lang-row[data-lang-select="zh"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh");
  await expect(page.locator("#drawerTitle")).toHaveText("设置");
  await expect(page.locator("#settingsBtn")).toContainText("设置");
  await expect(page.locator("#drawerBackBtn")).toBeHidden();
  await expect(page.locator('[data-settings-view="language"]')).toBeFocused();

  await page.locator('[data-settings-view="language"]').click();
  await expect(page.locator('.lang-row[data-lang-select="zh"]')).toHaveClass(/is-active/);
  await expect(page.locator('.lang-row[data-lang-select="zh"]')).toHaveAttribute("aria-checked", "true");
  await expect(page.locator('.lang-row[data-lang-select="en"]')).not.toHaveClass(/is-active/);
  await page.locator('.lang-row[data-lang-select="en"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.locator("#drawerTitle")).toHaveText("Settings");
  await expect(page.locator("#settingsBtn")).toContainText(/settings/i);
  await expect(page.locator("#drawerBackBtn")).toBeHidden();
});
