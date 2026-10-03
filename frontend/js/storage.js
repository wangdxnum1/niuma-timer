async function loadAutostart() {
  try {
    $("launch_on_boot").checked = await invoke("get_autostart");
  } catch (e) {
    flog("get_autostart ERR: " + (e && e.message ? e.message : String(e)));
  }
}

$("launch_on_boot").addEventListener("change", async () => {
  const want = $("launch_on_boot").checked;
  try {
    const msg = await invoke("set_autostart", { enabled: want });
    showToast(msg, "ok");
  } catch (e) {
    // 写注册表失败要把开关拨回真实状态，否则界面显示与实际不符
    await loadAutostart();
    const detail = e && e.message ? e.message : String(e);
    flog("set_autostart ERR: " + detail);
    showToast("设置失败：" + detail, "err");
  }
});

$("refreshBtn").addEventListener("click", refresh);

// ---- 数据存储：占用展示与立即整理 ----
function fmtBytes(n) {
  const v = Number(n) || 0;
  if (v < 1024) return v + " B";
  if (v < 1024 * 1024) return (v / 1024).toFixed(1) + " KB";
  return (v / 1024 / 1024).toFixed(2) + " MB";
}

// 占比文案：极小的项（<0.1%）不能显示成 0.0%，否则看起来像没占空间
function stgPct(bytes, total) {
  if (!(total > 0)) return "0%";
  const p = ((Number(bytes) || 0) / total) * 100;
  if (p <= 0) return "0%";
  if (p < 0.1) return "<0.1%";
  return p.toFixed(1) + "%";
}

// 上次读到的总占用，整理后据此算出释放了多少（WAL 归零是主要收益）
let stgLastTotal = null;

function renderStorageInfo(info) {
  const el = $("storageInfo");
  if (!el || !info) return;
  // 后端结构体是 snake_case 序列化（Tauri 不转驼峰，见 overtime 的 cross_midnight），
  // 这里写成 totalBytes 会静默拿到 undefined → 总计 0 B、每行占比全 0
  const total = Number(info.total_bytes) || 0;
  stgLastTotal = total;
  // 0 字节的项不展示：空库时不该列一堆 0 出来占版面
  const slices = (info.slices || []).filter((s) => s.bytes > 0);
  if (!slices.length) {
    el.innerHTML = '<span class="hint">暂无占用数据</span>';
    return;
  }
  let h =
    '<div class="stg-total"><span>共占用</span><b>' +
    fmtBytes(total) +
    "</b></div>";
  h += '<div class="stg-bar">';
  slices.forEach(function (s) {
    h +=
      '<i class="stg-seg k-' +
      s.key +
      '" style="width:' +
      (total > 0 ? (s.bytes / total) * 100 : 0) +
      '%"></i>';
  });
  h += "</div><ul class=\"stg-list\">";
  slices.forEach(function (s) {
    h +=
      '<li><i class="stg-dot k-' +
      s.key +
      '"></i><span class="stg-name">' +
      s.label +
      '</span><span class="stg-size">' +
      fmtBytes(s.bytes) +
      '</span><span class="stg-pct">' +
      stgPct(s.bytes, total) +
      "</span>" +
      (s.rows && s.unit
        ? '<span class="stg-sub">' + s.rows + " " + s.unit + "</span>"
        : "") +
      "</li>";
  });
  h += "</ul>";
  if (info.approx) h += '<p class="stg-note">表级占用按行数比例估算</p>';
  if (info.earliest_date) {
    h += '<p class="stg-note">最早数据 ' + info.earliest_date + "</p>";
  }
  el.innerHTML = h;
}

async function loadStorageInfo() {
  try {
    renderStorageInfo(await invoke("get_storage_info"));
  } catch (e) {
    flog("get_storage_info ERR: " + (e && e.message ? e.message : String(e)));
    const el = $("storageInfo");
    if (el) el.textContent = "占用信息读取失败";
  }
}

async function runCleanup() {
  const btn = $("cleanupBtn");
  if (btn) { btn.disabled = true; btn.textContent = "整理中…"; }
  try {
    const info = await invoke("run_maintenance");
    const after = Number(info && info.total_bytes) || 0;
    // 报「释放了多少」而不是「现在多少」：整理后 WAL 归零，说现有大小看着像什么都没做
    const freed =
      stgLastTotal !== null && stgLastTotal > after ? stgLastTotal - after : 0;
    renderStorageInfo(info);
    showToast(
      freed > 0
        ? "整理完成，释放 " + fmtBytes(freed) + "（当前共 " + fmtBytes(after) + "）"
        : "整理完成，已无可以释放的空间",
      "ok"
    );
  } catch (e) {
    const detail = e && e.message ? e.message : String(e);
    flog("run_maintenance ERR: " + detail);
    showToast("整理失败：" + detail, "err");
  } finally {
    if (btn) { btn.disabled = false; btn.textContent = "立即整理"; }
  }
}

$("retention_days").addEventListener("change", saveNow);
$("cleanupBtn").addEventListener("click", runCleanup);
// ---- 数据备份与还原 ----
// 低频重操作：与存储占用同模式，load() 时拉一次列表、操作后就地刷新，绝不进 tick 轮询。
async function loadBackups() {
  try {
    const list = await invoke("list_backups");
    renderBackups(list);
    const hint = $("backupDirHint");
    if (hint && list.length > 0 && list[0].path) {
      // 用第一条的目录回显实际备份落点（文档目录可能被 OneDrive 重定向）
      hint.textContent = "备份目录：" + list[0].path.slice(0, list[0].path.lastIndexOf("\\") + 1);
    }
  } catch (e) {
    flog("list_backups ERR: " + (e && e.message ? e.message : String(e)));
    const el = $("backupList");
    if (el) el.innerHTML = '<span class="hint">备份列表读取失败</span>';
  }
}

function renderBackups(list) {
  const box = $("backupList");
  box.textContent = "";
  if (!list || list.length === 0) {
    box.textContent = "还没有备份";
    return;
  }
  list.forEach((b) => {
    const row = document.createElement("div");
    row.className = b.broken ? "backup-row broken" : "backup-row";
    const button = document.createElement("button");
    button.disabled = !!b.broken;
    button.textContent = "还原";
    button.dataset.backup = b.name;
    const label = document.createElement("span");
    label.textContent = b.broken ? b.name + " · 不可用" :
      b.created_at + " · v" + b.app_version + " · " + fmtBytes(b.bytes);
    row.append(label, button);
    box.append(row);
  });
}

async function doBackupNow() {
  const btn = $("backupNowBtn");
  if (btn) {
    btn.disabled = true;
    btn.textContent = "备份中…";
  }
  try {
    const entry = await invoke("backup_now");
    showToast("备份完成：" + entry.name, "ok");
    const st = $("backupStatus");
    if (st) {
      st.textContent = "最近备份：" + entry.path + "（" + fmtBytes(entry.bytes) + "）";
    }
    await loadBackups();
  } catch (e) {
    const detail = e && e.message ? e.message : String(e);
    flog("backup_now ERR: " + detail);
    showToast("备份失败：" + detail, "err");
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = "立即备份";
    }
  }
}

let restoreBusy = false;
async function doRestore(name) {
  if (restoreBusy) return;
  restoreBusy = true;
  // 二次确认必须说清后果与可逆性：覆盖什么 + 当前数据会被自动保护
  const yes = await showConfirm(
    "将用这份备份覆盖当前全部数据（加班 / 活动 / 应用 / 媒体 / 设置）。当前数据会先自动备份一份。确定继续？"
  );
  if (!yes) { restoreBusy = false; return; }
  try {
    await invoke("restore_backup", { name });
    // 成功即盖不可取消遮罩：后端 800ms 后 restart，窗口内禁止任何继续操作
    $("restoreMask").classList.remove("hidden");
  } catch (e) {
    // 失败不重启、当前数据完好，红字给原因
    const detail = e && e.message ? e.message : String(e);
    flog("restore_backup ERR: " + detail);
    showToast("还原失败：" + detail, "err");
    restoreBusy = false;
  }
}
$("backupNowBtn").addEventListener("click", doBackupNow);
$("backupList").addEventListener("click", (e) => {
  const btn = e.target.closest("button[data-backup]");
  if (btn) doRestore(btn.dataset.backup);
});
// 加班记录增删改
$("otAddBtn").addEventListener("click", openOtForm);
$("otExportBtn").addEventListener("click", exportOvertimeCsv);
$("otPrevMonth").addEventListener("click", () => shiftOtMonth(-1));
$("otNextMonth").addEventListener("click", () => shiftOtMonth(1));
$("otfSave").addEventListener("click", submitOtForm);
$("otfCancel").addEventListener("click", hideOtForm);