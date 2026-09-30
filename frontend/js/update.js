// ====== 自动更新（v1.4.0）======
// updateInfo：最近一次 check_update 的原始结果；updateAnnounce：升级后公告（一次性）
let updateInfo = null;
let updateAnnounce = null;
let updateChecking = false;
let updateSettingsScroll = 0;

// 最近一次 update-progress 事件；null = 无进行中的更新
let updateProgress = null;

// 把 CHANGELOG 片段的 markdown 源码转成安全的 HTML：先整体转义再还原有限标记，
// 支持 标题/列表/围栏代码块/粗体/行内代码；链接只展示文字 + title，不可点击，
// 避免更新说明里的外链把用户带离应用。
function renderMarkdown(text) {
  if (!text || !text.trim()) return "暂无更新说明";
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const inline = (s) => esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<span class="upd-link" title="$2">$1</span>');
  const html = [];
  let list = null;
  let code = false;
  const closeList = () => {
    if (list) { html.push("</" + list + ">"); list = null; }
  };
  for (const raw of String(text).split(/\r?\n/)) {
    if (raw.trim().startsWith("```")) {
      if (code) { html.push("</code></pre>"); code = false; }
      else { closeList(); html.push("<pre><code>"); code = true; }
      continue;
    }
    if (code) { html.push(esc(raw)); continue; } // 围栏内容同样先转义，防止裸 HTML 成真标签
    const t = raw.trim();
    if (!t) { closeList(); continue; }
    if (t.startsWith("### ")) { closeList(); html.push("<h5>" + inline(t.slice(4)) + "</h5>"); continue; }
    if (t.startsWith("## ")) { closeList(); html.push("<h4>" + inline(t.slice(3)) + "</h4>"); continue; }
    const ul = t.match(/^[-*] (.*)$/);
    if (ul) {
      if (list !== "ul") { closeList(); html.push("<ul>"); list = "ul"; }
      html.push("<li>" + inline(ul[1]) + "</li>");
      continue;
    }
    const ol = t.match(/^\d+\. (.*)$/);
    if (ol) {
      if (list !== "ol") { closeList(); html.push("<ol>"); list = "ol"; }
      html.push("<li>" + inline(ol[1]) + "</li>");
      continue;
    }
    closeList();
    html.push("<p>" + inline(t) + "</p>");
  }
  closeList();
  if (code) html.push("</code></pre>");
  return html.join("\n");
}

function returnFromUpdate() {
  showView("viewSettings");
  $("viewSettings").scrollTop = updateSettingsScroll;
  $("settingsCheckUpdateBtn").focus({ preventScroll: true });
}

// 画更新页：公告优先且只显示一次，其余按检查结果显示
function paintUpdate() {
  const info = updateInfo;
  $("updCurrent").textContent = info && info.current ? "v" + info.current : "—";
  $("updLatest").textContent =
    !updateChecking && info && !info.error && info.latest ? "v" + info.latest : "—";
  const hasUpdate = !!(!updateChecking && info && info.has_update && !info.error);
  $("updLatestCard").classList.toggle("has-update", hasUpdate);
  $("updCheckBtn").disabled = updateChecking;
  $("updCheckBtn").setAttribute("aria-busy", String(updateChecking));
  $("updCheckLabel").textContent = updateChecking ? "检查中…" : info && info.error ? "重新检查" : "检查更新";
  $("updStatus").dataset.state = updateChecking ? "checking" : info && info.error ? "error" : hasUpdate ? "available" : info ? "current" : "idle";
  if (updateChecking) {
    $("updStatus").textContent = "正在获取最新版本信息…";
  } else if (info && info.error) {
    $("updStatus").textContent = "检查失败：" + info.error;
  } else if (info && info.has_update) {
    $("updStatus").textContent = info.installed
      ? "有新版本，可一键更新"
      : "有新版本，可自动替换便携版并重启";
  } else if (info) {
    $("updStatus").textContent = "已是最新版本 v" + info.current;
  } else {
    $("updStatus").textContent = "点击「检查更新」获取最新版本";
  }
  $("updNotes").innerHTML = renderMarkdown(info && info.notes ? info.notes : "");
  $("updSkipBtn").disabled = !hasUpdate;
  $("updSkipBtn").classList.toggle("hidden", !hasUpdate);
  // 公告覆盖在检查结果之上，展示过即清空（后端也只会下发一次）
  if (updateAnnounce && !updateChecking) {
    $("updStatus").textContent = "已更新到当前版本";
    $("updNotes").innerHTML = renderMarkdown(updateAnnounce);
    updateAnnounce = null;
  }
  // 仅在检查完成且有新版本时提供更新操作。
  $("updUpdateBtn").classList.toggle(
    "hidden",
    !hasUpdate
  );
}

// 画下载进度：由 update-progress 事件驱动；retrying/error 保持上次的进度条位置
function paintUpdateProgress() {
  const box = $("updProgress");
  const fill = $("updProgressFill");
  const text = $("updProgressText");
  if (!updateProgress || !updateProgress.phase) {
    box.classList.add("hidden");
    return;
  }
  box.classList.remove("hidden");
  const mb = (n) => (n / 1048576).toFixed(1);
  const p = updateProgress;
  if (p.phase === "downloading") {
    if (p.total) {
      const pct = Math.min(100, Math.round((p.downloaded / p.total) * 100));
      box.classList.remove("indeterminate");
      fill.style.width = pct + "%";
      text.textContent = "正在下载… " + pct + "%（" + mb(p.downloaded) + " / " + mb(p.total) + " MB）";
    } else {
      box.classList.add("indeterminate");
      fill.style.width = "";
      text.textContent = "正在下载… " + mb(p.downloaded) + " MB";
    }
  } else if (p.phase === "retrying") {
    text.textContent = "网络中断，正在重试（第 " + p.attempt + " 次）…";
  } else if (p.phase === "verifying") {
    box.classList.remove("indeterminate");
    fill.style.width = "100%";
    text.textContent = "下载完成，正在校验完整性…";
  } else if (p.phase === "installing") {
    box.classList.remove("indeterminate");
    fill.style.width = "100%";
    text.textContent = updateInfo && updateInfo.installed
      ? "正在安装，安装程序将自动关闭本程序…"
      : "正在替换程序文件，即将自动重启…";
  } else if (p.phase === "restarting") {
    fill.style.width = "100%";
    text.textContent = "正在重启…";
  } else if (p.phase === "error") {
    text.textContent = "更新失败：" + (p.message || "未知错误");
  }
}

async function loadUpdateInfo() {
  if (updateChecking) return;
  updateChecking = true;
  paintUpdate();
  try {
    updateInfo = await invoke("check_update");
  } catch (e) {
    updateInfo = {
      current: updateInfo && updateInfo.current,
      has_update: false,
      installed: false,
      error: e && e.message ? e.message : String(e),
    };
  } finally {
    updateChecking = false;
    paintUpdate();
  }
}

// 托盘菜单「检查更新」→ 主窗打开更新页（事件由 tray.rs 广播）
async function watchUpdateView() {
  try {
    await TAURI.event.listen("update-view-requested", () => showView("viewUpdate"));
  } catch (err) {
    flog("update view listen failed: " + (err && err.message ? err.message : String(err)));
  }
}

// 订阅后端下载进度；事件系统若不通则进度条停留在最近一次画面——不阻塞更新本身
async function watchUpdateProgress() {
  try {
    await TAURI.event.listen("update-progress", (evt) => {
      updateProgress = evt && evt.payload ? evt.payload : null;
      if (curView === "viewUpdate") paintUpdateProgress();
    });
  } catch (err) {
    flog("update progress listen failed: " + (err && err.message ? err.message : String(err)));
  }
}

$("updCheckBtn").addEventListener("click", loadUpdateInfo);
// 设置页「检查更新」= 进更新页；取数由 showView 收尾统一触发，这里不重复发一次请求
$("settingsCheckUpdateBtn").addEventListener("click", () => {
  updateAnnounce = null; // 手动检查时不再展示上次的升级公告
  showView("viewUpdate");
});
$("updBackBtn").addEventListener("click", returnFromUpdate);
$("updSkipBtn").addEventListener("click", async () => {
  if (!updateInfo || !updateInfo.latest) return;
  try {
    await invoke("skip_update_version", { version: updateInfo.latest });
    showToast("已跳过 v" + updateInfo.latest, "ok");
    showView("viewMain");
  } catch (e) {
    showToast("跳过失败：" + (e && e.message ? e.message : String(e)), "err");
  }
});
// 立即更新：进度由 update-progress 事件驱动；失败时窗口不再消失，可直接看到原因并重试
$("updUpdateBtn").addEventListener("click", async () => {
  const button = $("updUpdateBtn");
  if (button.disabled) return;
  button.disabled = true;
  updateProgress = { phase: "downloading", downloaded: 0, total: 0, attempt: 1 };
  paintUpdateProgress();
  try {
    await invoke("start_update");
  } catch (e) {
    updateProgress = { phase: "error", message: e && e.message ? e.message : String(e) };
    paintUpdateProgress();
    button.disabled = false;
  }
});
