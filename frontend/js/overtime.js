function applyOvertimeVisibility(enabled) {
  const card = $("otCard");
  if (card) card.style.display = enabled ? "" : "none";
}

// 休息日/节假日加班关闭时隐藏其三项子配置，避免看到一堆不生效的输入框
function applyRestOvertimeVisibility(enabled) {
  const box = $("restOtFields");
  if (box) box.style.display = enabled ? "" : "none";
}

// 加班明细当前查看的年月；null = 跟随当月
let otView = null;

// 加班记录加载与渲染
// 懒渲染契约（同 monitor/insights）：明细表只在加班页可见时重绘——
// 此前 10 秒轮询停在主页也全量重建隐藏表格、还多拉一份浏览月的 IPC。
// 主界面「加班战果」卡走单独的轻路径，只拉当月。
async function loadOvertime() {
  // 加班追踪关闭时不拉取、不展示（历史记录仍保留在 SQLite，开关不影响数据）
  if (!$("overtime_enabled").checked) return;
  const now = new Date();
  const curY = now.getFullYear();
  const curM = now.getMonth() + 1;
  try {
    if (curView === "viewOt") {
      const vy = otView ? otView.year : curY;
      const vm = otView ? otView.month : curM;
      const ot = await invoke("get_overtime_records", { year: vy, month: vm });
      renderOtTable(ot);
      // 主界面「加班总览」固定反映当月：翻到历史月份时要单独拉当月，
      // 否则主界面会显示 8 月的合计却标着「本月合计」，属于数据错位。
      if (vy === curY && vm === curM) {
        renderOtHome(ot);
      } else {
        renderOtHome(await invoke("get_overtime_records", { year: curY, month: curM }));
      }
    } else {
      // 不在加班页：只维护主界面当月卡，明细表不碰
      renderOtHome(await invoke("get_overtime_records", { year: curY, month: curM }));
    }
  } catch (e) {
    // 与 monitor/bill 同标准：失败要可见。写进小计区（成功重绘会整体重画），
    // 轮询场景写 DOM 文本不会像 toast 那样轰炸。
    if (curView === "viewOt") {
      const sum = $("otMonthSummary");
      if (sum) {
        sum.classList.add("empty");
        sum.textContent = "加班明细加载失败（详见 debug.log）";
      }
    }
    console.error("loadOvertime error:", e);
  }
}

// 翻月：delta = -1 上一月 / +1 下一月
function shiftOtMonth(delta) {
  const now = new Date();
  const base = otView || { year: now.getFullYear(), month: now.getMonth() + 1 };
  let y = base.year;
  let m = base.month + delta;
  if (m < 1) {
    m = 12;
    y -= 1;
  } else if (m > 12) {
    m = 1;
    y += 1;
  }
  otView = { year: y, month: m };
  loadOvertime();
}

// 主界面「本月加班战果」金色卡：只吃当月数据（¥ 符号在模板里，这里只写数值）
function renderOtHome(ot) {
  $("otMonthTotal").textContent = ot.total_all.toFixed(0);
  $("otMonthHours").textContent = ot.total_hours.toFixed(1) + "h";
  $("otMonthDays").textContent = ot.days + " 天";
  $("otMonthAvg").textContent =
    "¥" + (ot.days > 0 ? (ot.total_all / ot.days).toFixed(0) : "0");
}

// 加班明细页：表格 + 所选月份小计，跟随 otView
let lastOt = null; // 当前展示月份的加班数据缓存，供导出 CSV 使用
function renderOtTable(ot) {
  lastOt = ot;
  $("otMonthLabel").textContent = ot.year + " 年 " + ot.month + " 月";
  const sum = $("otMonthSummary");
  if (sum) {
    if (ot.records.length) {
      sum.classList.remove("empty");
      const avg = ot.days > 0 ? ot.total_all / ot.days : 0;
      sum.innerHTML =
        '<div class="os-amount">¥' + ot.total_all.toFixed(0) + '</div>' +
        '<div class="os-meta">' +
          '<div class="cell"><div class="k">有效时长</div><div class="v">' +
            ot.total_hours.toFixed(1) + 'h</div></div>' +
          '<div class="cell"><div class="k">加班天数</div><div class="v">' +
            ot.days + ' 天</div></div>' +
          '<div class="cell"><div class="k">日均进账</div><div class="v">¥' +
            avg.toFixed(0) + '</div></div>' +
        '</div>';
    } else {
      sum.classList.add("empty");
      sum.textContent = "该月暂无加班记录";
    }
  }
  // 下一月按钮翻到当前月为止：未来没有加班记录
  const now = new Date();
  const curY = now.getFullYear();
  const curM = now.getMonth() + 1;
  $("otNextMonth").disabled =
    ot.year > curY || (ot.year === curY && ot.month >= curM);
  const tbody = $("ot_tbody");
  tbody.innerHTML = "";
  if (ot.records.length === 0) {
    tbody.innerHTML =
      '<tr><td colspan="8" class="ot-empty">暂无加班记录</td></tr>';
    return;
  }
  for (const r of [...ot.records].reverse()) {
    const tr = document.createElement("tr");
    // 来源：1 = 手动录入（不会被自动锁屏记录覆盖），0 = 自动
    const isManual = r.source === 1;
    const cells = [
      r.date.slice(5),
      // 跨午夜的离开时刻是次日凌晨，光看 "01:30" 会被误读成当天凌晨
      r.cross_midnight ? "次日 " + r.lock_time : r.lock_time,
      r.valid_hours.toFixed(1) + "h",
      "¥" + r.fee.toFixed(0),
      r.meal > 0 ? "¥" + r.meal.toFixed(0) : "—",
      "¥" + r.total.toFixed(0),
      isManual ? "手动" : "自动",
    ];
    cells.forEach((c, i) => {
      const td = document.createElement("td");
      td.textContent = c;
      if (i === cells.length - 1) {
        td.className = isManual ? "ot-src manual" : "ot-src";
      }
      tr.appendChild(td);
    });
    const opTd = document.createElement("td");
    opTd.className = "ot-op";
    const editBtn = document.createElement("button");
    editBtn.textContent = "编辑";
    editBtn.className = "btn-sm";
    editBtn.onclick = () => editOtRecord(r);
    opTd.appendChild(editBtn);
    const delBtn = document.createElement("button");
    delBtn.textContent = "删除";
    delBtn.className = "btn-sm danger";
    delBtn.onclick = () => deleteOtRecord(r.date);
    opTd.appendChild(delBtn);
    tr.appendChild(opTd);
    tbody.appendChild(tr);
  }
}

// ---- 导出 CSV：数据虽已在 JS 侧，仍走后端 export_csv 写「下载」目录 ----
// （后端负责文件名清洗与 UTF-8 BOM，前端负责按钮反馈与并发守卫，见 downloadCsv）
// RFC4180 转义：含逗号/引号/换行才包双引号，内部引号翻倍
function csvCell(v) {
  const s = v == null ? "" : String(v);
  if (/[",\n\r]/.test(s)) return '"' + s.replace(/"/g, '""') + '"';
  return s;
}
function csvRows(rows) {
  return rows.map((r) => r.map(csvCell).join(",")).join("\r\n");
}
// 导出 CSV：经后端 export_csv 命令写盘到「下载」目录（BOM 由后端写入）。
// 反馈走现有 showToast（窗口顶部居中气泡，非模态）；加并发守卫，避免连点刷出一堆同名文件。
let exportingCsv = false;
async function downloadCsv(filename, csv) {
  if (exportingCsv) return;
  exportingCsv = true;
  try {
    await invoke("export_csv", { filename: filename, content: csv });
    showToast("已导出：" + filename, "ok");
  } catch (e) {
    showToast("导出失败：" + (e && e.message ? e.message : e), "err");
  } finally {
    exportingCsv = false;
  }
}
function exportOvertimeCsv() {
  const ot = lastOt;
  if (!ot || !ot.records || ot.records.length === 0) return;
  const rows = [
    ["日期", "是否跨午夜", "锁屏/下班时刻", "有效时长(小时)", "加班费(元)", "餐补(元)", "合计(元)", "来源"],
  ];
  for (const r of [...ot.records].sort((a, b) => (a.date < b.date ? -1 : 1))) {
    rows.push([
      r.date,
      r.cross_midnight ? "是" : "否",
      r.cross_midnight ? "次日 " + r.lock_time : r.lock_time,
      r.valid_hours,
      r.fee,
      r.meal,
      r.total,
      r.source === 1 ? "手动" : "自动",
    ]);
  }
  rows.push([]);
  rows.push(["（月度汇总）"]);
  rows.push(["月份", ot.year + "-" + String(ot.month).padStart(2, "0")]);
  rows.push(["合计金额(元)", ot.total_all]);
  rows.push(["总有效时长(小时)", ot.total_hours]);
  rows.push(["加班天数", ot.days]);
  downloadCsv(
    "加班明细_" + ot.year + "-" + String(ot.month).padStart(2, "0") + ".csv",
    csvRows(rows)
  );
}
function exportWeekBillCsv() {
  const bill = billData;
  if (!bill || !bill.buckets || bill.buckets.length === 0) return;
  const rows = [["（汇总）"]];
  rows.push(["周期", bill.period_label]);
  rows.push(["起始日期", bill.period_start]);
  rows.push(["结束日期", bill.period_end]);
  rows.push(["总进账(元)", bill.total_income]);
  rows.push(["基本工资(元)", bill.base_salary]);
  rows.push(["加班费(元)", bill.ot_fee]);
  rows.push(["摸鱼成本(元)", bill.slack_cost]);
  rows.push(["出勤工时(小时)", bill.work_hours]);
  rows.push(["出勤天数", bill.work_days]);
  rows.push(["键鼠次数", bill.keys_total]);
  rows.push(["点击次数", bill.clicks_total]);
  rows.push([]);
  rows.push(["（分期明细）"]);
  rows.push(["日期", "名称", "是否工作日", "有记录", "当日工资(元)", "摸鱼秒数"]);
  for (const d of bill.buckets) {
    rows.push([
      d.date || "",
      d.label || "",
      d.is_workday == null ? "" : d.is_workday ? "是" : "否",
      d.has_record ? "是" : "否",
      d.salary,
      d.slack_seconds,
    ]);
  }
  downloadCsv(
    "账单_" + (bill.period_start || "") + ".csv",
    csvRows(rows)
  );
}

// ---- 加班记录手动增删改（仅当月）----
function showOtForm() {
  $("otForm").classList.remove("hidden");
}
function hideOtForm() {
  $("otForm").classList.add("hidden");
  $("otfMsg").textContent = "";
}
function showOtMsg(msg) {
  $("otfMsg").textContent = msg;
}

// 点击「添加记录」：自动预填今天日期 + 当前时刻，焦点落在下班时间上
function openOtForm() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  $("otf_date").value =
    d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
  $("otf_lock").value = pad(d.getHours()) + ":" + pad(d.getMinutes());
  $("otf_start").value = "";
  $("otf_cross").checked = false;
  $("otfTitle").textContent = "添加加班记录";
  $("otfSave").textContent = "保存";
  showOtMsg("");
  showOtForm();
  $("otf_lock").focus();
}

// 点击某行的「编辑」
function editOtRecord(r) {
  $("otf_date").value = r.date;
  $("otf_lock").value = r.lock_time;
  $("otf_start").value = r.ot_start && r.ot_start !== "" ? r.ot_start : "";
  $("otf_cross").checked = !!r.cross_midnight;
  $("otfTitle").textContent = "编辑加班记录";
  $("otfSave").textContent = "更新";
  showOtMsg("");
  showOtForm();
  $("otf_lock").focus();
}

// 提交添加/更新（后端按日期 upsert，同一日期即覆盖=编辑）
async function submitOtForm() {
  const date = $("otf_date").value;
  const lock = $("otf_lock").value;
  const start = $("otf_start").value || null;
  // 嵌套 input 按 serde 原样反序列化（只有顶层命令参数才转 camelCase），
  // 字段必须与 ManualOvertimeInput 一致：写成 crossMidnight 会被丢掉，永远是 false。
  const cross_midnight = $("otf_cross").checked;
  if (!date || !lock) {
    showOtMsg("请填写日期和下班时间");
    return;
  }
  // 前端先把关：不能是未来日期。历史月份允许补录——界面已能切月查看，
  // 再把写入锁死在当月就没法补记忘掉的加班了。
  const picked = new Date(date + "T00:00:00");
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  if (picked > today) {
    showOtMsg("不能添加未来日期的加班记录");
    return;
  }
  try {
    const view = await invoke("save_overtime_record", {
      input: { date, lock_time: lock, ot_start: start, cross_midnight },
    });
    // 跟到记录所属月份：补录 8 月时视图停在 8 月，不会莫名跳回当月
    otView = { year: view.year, month: view.month };
    hideOtForm();
    showToast("已保存加班记录", "ok");
    loadOvertime();
  } catch (e) {
    showOtMsg("保存失败：" + e);
  }
}

// 自定义确认弹窗（替代浏览器默认 confirm）
let confirmResolve = null;
function showConfirm(text) {
  return new Promise((resolve) => {
    confirmResolve = resolve;
    $("confirmText").textContent = text;
    $("confirmModal").classList.remove("hidden");
  });
}
function hideConfirm(result) {
  $("confirmModal").classList.add("hidden");
  if (confirmResolve) {
    confirmResolve(result);
    confirmResolve = null;
  }
}
$("confirmOk").addEventListener("click", () => hideConfirm(true));
$("confirmCancel").addEventListener("click", () => hideConfirm(false));

// 删除某天记录
async function deleteOtRecord(date) {
  const ok = await showConfirm("确定删除 " + date + " 的加班记录？");
  if (!ok) return;
  try {
    const view = await invoke("delete_overtime_record", { date });
    otView = { year: view.year, month: view.month };
    loadOvertime();
    showToast("已删除", "ok");
  } catch (e) {
    showToast("删除失败：" + e, "err");
  }
}

// ---- 活动统计（鼠标/键盘）----

// 像素 → 可读距离：96dpi 下 1 英寸 = 96px，1 米 ≈ 3779.5px
