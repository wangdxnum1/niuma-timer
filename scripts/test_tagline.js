// 首页副标题文案的回归测试：从 app.js 抽出真实函数，用桩 DOM 跑所有状态分支。
// 用法（任意目录）：node scripts/test_tagline.js
const fs = require("fs");
const path = require("path");
const src = fs.readFileSync(path.join(__dirname, "..", "frontend", "app.js"), "utf8");

function pick(re, name) {
  const m = src.match(re);
  if (!m) {
    console.error("提取失败: " + name);
    process.exit(1);
  }
  return m[0];
}

const code = [
  pick(/function minutesOf\(hhmm\) \{[\s\S]*?\n\}/, "minutesOf"),
  pick(/function dynamicTagline\(s\) \{[\s\S]*?\n\}/, "dynamicTagline"),
  pick(/const FIXED_TAGLINES = \{[\s\S]*?\n\};/, "FIXED_TAGLINES"),
  pick(/function taglineStyle\(\) \{[\s\S]*?\n\}/, "taglineStyle"),
  pick(/function renderTagline\(s\) \{[\s\S]*?\n\}/, "renderTagline"),
].join("\n");

let store = { am_end: "12:00", pm_start: "13:00", tagline_style: "dynamic" };
let out = "";
let hidden = false;

global.$ = (id) => ({
  get value() {
    return store[id] !== undefined ? store[id] : "";
  },
  set textContent(v) {
    out = v;
  },
  get textContent() {
    return out;
  },
  style: {
    set display(v) {
      hidden = v === "none";
    },
  },
});

// 包进函数里 eval：否则 function 声明会落进模块顶层作用域，
// 与下面的同名 const 撞车（Identifier has already been declared）
function loadApi() {
  return eval(code + "\n;({ dynamicTagline, renderTagline, taglineStyle })");
}
const { dynamicTagline, renderTagline, taglineStyle } = loadApi();

let pass = 0;
let fail = 0;
function check(desc, actual, expect) {
  if (actual === expect) {
    pass++;
    console.log("  PASS " + desc + "  ->  " + actual);
  } else {
    fail++;
    console.log("  FAIL " + desc + "\n       期望: " + expect + "\n       实际: " + actual);
  }
}

const work = (o) =>
  Object.assign(
    { to_off_str: "3小时0分0秒", off_work: false, worked_h: 4, to_off_h: 3 },
    o
  );

console.log("动态状态分支：");
check("休息日", dynamicTagline(work({ to_off_str: "今天休息" })), "今天休息，钱也休息");
check("已下班", dynamicTagline(work({ off_work: true, to_off_h: 0 })), "今天的钱，就到这儿了");
check("未开工", dynamicTagline(work({ worked_h: 0 })), "还没开工，钱暂时没动");

store.am_end = "00:00";
store.pm_start = "23:59";
check("午休（区间覆盖当前时刻）", dynamicTagline(work()), "午休中，钱先歇会儿");
store.am_end = "12:00";
store.pm_start = "13:00";

check("剩 30 分钟（边界）", dynamicTagline(work({ to_off_h: 0.5 })), "还有 30 分钟，撑住");
check("剩 15 分钟", dynamicTagline(work({ to_off_h: 0.25 })), "还有 15 分钟，撑住");
check("剩 1 分钟", dynamicTagline(work({ to_off_h: 0.01 })), "还有 1 分钟，撑住");
check("剩 31 分钟（不算临近）", dynamicTagline(work({ to_off_h: 0.51 })), "正在搬砖，钱一直在涨");
check("工作中", dynamicTagline(work({ to_off_h: 3 })), "正在搬砖，钱一直在涨");

console.log("\n风格分支：");
store.tagline_style = "price";
renderTagline(work());
check("price", out, "你今天的每一分钟，都明码标价");
store.tagline_style = "rise";
renderTagline(work());
check("rise", out, "每一秒，钱都在涨");
store.tagline_style = "count";
renderTagline(work());
check("count", out, "搬砖的每一分钟，都算数");
store.tagline_style = "classic";
renderTagline(work());
check("classic（原版）", out, "实时计算你今天赚了多少钱");

store.tagline_style = "none";
renderTagline(work());
check("none 隐藏该行", hidden, true);

store.tagline_style = "custom";
store.tagline_custom = "今天也要好好摸鱼";
renderTagline(work());
check("custom 自定义", out, "今天也要好好摸鱼");
store.tagline_custom = "   ";
renderTagline(work());
check("custom 留空回退动态句", out, "正在搬砖，钱一直在涨");

store.tagline_style = "";
check("控件未填好时 taglineStyle 兜底", taglineStyle(), "dynamic");

console.log("\n" + pass + " passed, " + fail + " failed");
process.exit(fail ? 1 : 0);
