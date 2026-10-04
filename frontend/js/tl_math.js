// 今日时间轴的共享数学（hero 主界面与 hover_card 悬停卡两个文档共用）。
// 此前同一套 HH:MM→分钟、区间重叠、按跨度取百分比在两个文档各抄一份，
// 已经漂出第三个副本——任何口径调整只改这里。
// 注：DOM 渲染两文档自理（配置来源与元素访问不同），这里只收纯函数。

// "HH:MM" → 当天分钟数；解析失败返 null（调用方以 null 判配置异常并隐藏整块）
function minutesOf(hhmm) {
  const p = String(hhmm || "").split(":");
  if (p.length !== 2) return null;
  const h = parseInt(p[0], 10);
  const m = parseInt(p[1], 10);
  if (isNaN(h) || isNaN(m)) return null;
  return h * 60 + m;
}

// 落在 [s,e] 时段内的分钟数（与后端 calc::overlap 同口径）
function overlapMin(t, s, e) {
  return t <= s ? 0 : t >= e ? e - s : t - s;
}

// v 相对 0..span 的百分比宽度（钳到段内），返回带 % 的字符串
function spanPct(v, span) {
  return (Math.min(Math.max(v, 0), span) / span) * 100 + "%";
}
