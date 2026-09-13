// 摸鱼统计回归测试（应用分类：工作/摸鱼/沟通/其他）：
// 1) index.html：应用明细页有「今日构成」卡（catSummary 容器 + 说明文案）
// 2) app.js：分类循环表 CAT_CYCLE（四类固定序）、catKey 配色映射、
//    renderCategories 构成条渲染、cycleAppCategory 走 load_config→改 map→
//    save_config 的覆盖写路径、主页监控卡摸鱼速览行（mon-slack）
// 3) renderAppRows 支持 opts.chips 分类标签（仅明细页开启）
// 运行：node scripts/test_slacking.js
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const appSrc = fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8");
const html = fs.readFileSync(path.join(ROOT, "frontend", "index.html"), "utf8");
const css = fs.readFileSync(path.join(ROOT, "frontend", "styles.css"), "utf8");

let pass = 0;
let fail = 0;
function eq(label, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + label + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + label + "  ->  " + actual + "  (期望 " + expect + ")");
  }
}
function has(label, src, needle) {
  eq(label, src.includes(needle), true);
}

// ---------------------------------------------------------------- 1. index.html 结构
eq("明细页有分类汇总容器 catSummary", html.includes('id="catSummary"'), true);
has("构成卡说明文案（可点标签改分类）", html, "点下方应用行的分类标签可改");

// ---------------------------------------------------------------- 2. app.js 分类逻辑
has("分类循环表 CAT_CYCLE", appSrc, 'const CAT_CYCLE = ["工作", "摸鱼", "沟通", "其他"];');
has("catKey 配色映射", appSrc, "function catKey(label)");
has("构成条渲染 renderCategories", appSrc, "function renderCategories(s)");
has("明细页接入构成渲染", appSrc, "renderCategories(s);");
has("改分类走覆盖写路径", appSrc, "async function cycleAppCategory(app, current)");
has("改分类读取现有 map 再改", appSrc, "Object.assign({}, cfg.app_categories || {})");
has("改分类后保存", appSrc, 'invoke("save_config", { cfg: { app_categories: map } })');
has("改分类后刷新视图", appSrc, "loadAppUsage();");
// 主页监控卡摸鱼速览行
has("主页摸鱼速览行", appSrc, 'class="mon-slack"');
has("速览行引用工作构成", appSrc, 'c.key === "work"');
has("速览行引用摸鱼构成", appSrc, 'c.key === "slack"');
// 应用行分类标签（仅明细页 chips 模式）
has("renderAppRows 支持 chips 选项", appSrc, "opts.chips");
eq("chip 标注可点切换", appSrc.includes("点击切换分类"), true);
has("明细页开启 chips", appSrc, '{ chips: true }');

// ---------------------------------------------------------------- 3. 摸鱼成本 hero 烧钱行
// 3.1 index.html 结构：烧钱行四元素，且默认 hidden
eq("hero 卡有烧钱行容器", html.includes('id="slackBurn"'), true);
has("烧钱行默认 hidden", html, 'class="slack-burn hidden"');
eq("烧钱金额元素", html.includes('id="sbAmt"'), true);
eq("摸鱼率元素", html.includes('id="sbRate"'), true);
eq("损味文案元素", html.includes('id="sbQuip"'), true);
has("烧钱行位于 hero live 卡内", html, '<section class="card live">');

// 3.2 app.js：纯函数与四档文案
has("有四档文案表 SLACK_QUIPS", appSrc, "const SLACK_QUIPS");
has("有纯函数 slackQuip", appSrc, "function slackQuip(pct)");
has("文案：梦中情马", appSrc, "老板的梦中情马");
has("文案：装得敬业", appSrc, "摸得克制，装得敬业");
has("文案：班白上了", appSrc, "快三分之一的班白上了");
has("文案：注销公司", appSrc, "老板看完连夜注销公司");
has("有渲染函数 renderSlackBurn", appSrc, "function renderSlackBurn()");
has("tick() 触发烧钱行", appSrc, "renderSlackBurn();");

// 3.3 计算口径：成本 = 摸鱼秒 ÷3600 × 时薪；摸鱼率分母四类求和
has("成本公式（秒÷3600×时薪）", appSrc, "(slackSec / 3600) * st.hourly_rate");
has("摸鱼率公式（摸鱼/总时长）", appSrc, "(slackSec / total) * 100");
has("总时长为四类求和", appSrc, "cats.reduce((a, c) => a + (c.seconds || 0), 0)");
has("取 slack 切片", appSrc, 'c.key === "slack"');

// 3.4 三条显示守卫：工作日 / 时薪>0 / 总时长>0，不满足整块隐藏
has("守卫：工作日", appSrc, "!st.is_workday");
has("守卫：时薪大于 0", appSrc, "!(st.hourly_rate > 0)");
has("守卫：有记录总时长>0", appSrc, "total <= 0");
has("守卫失败整块隐藏", appSrc, 'box.classList.add("hidden")');
has("只取今日 appu（历史日期不进 hero）", appSrc, "isToday(appu.date)");
has("停用时同步隐藏", appSrc, "renderSlackBurn(); // 停用时 hero 烧钱行一并隐藏");

// 3.5 明细构成金额化
has("构成金额行 class", appSrc, "cat-money");
has("摸鱼类金额高亮按 key 拼接", appSrc, "cat-money cat-money-");
has("构成金额同口径公式", appSrc, '((c.seconds / 3600) * rate).toFixed(2)');

// 3.6 styles.css 样式
has("CSS：烧钱行容器", css, ".slack-burn {");
has("CSS：摸鱼红金额", css, ".sb-amt");
has("CSS：烧钱行可隐藏", css, ".slack-burn.hidden");
has("CSS：构成摸鱼金额红色", css, ".cat-money-slack");

// ---------------------------------------------------------------- 4. 兜底：$() 引用存在
const refIds = [...appSrc.matchAll(/\$\("([A-Za-z_]\w*)"\)/g)].map((m) => m[1]);
const missing = [...new Set(refIds)].filter((id) => !html.includes('id="' + id + '"'));
eq("app.js 引用的元素全部存在于 index.html", JSON.stringify(missing), "[]");

console.log("");
console.log(pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
