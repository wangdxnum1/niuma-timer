# 主页 hero 化：赚钱进度条 + 监控三合一 + 切换动效

- 日期：2026-09-12
- 状态：已与用户确认（Top 3 建议：进度条 / hero 化 / 切换动效）
- 依据：原生 Fluent 层级理念 + 突出「赚钱」软件身份（见当轮讨论）

## 目标

1. 主页确立信息层级：今日实时是明星（hero），监控是次要预览
2. 「今天的钱赚了几成」具象化——hero 卡内置金色赚钱进度条
3. 视图切换有原生手感（Fluent 式淡入），并尊重系统减弱动效设置

## 方案

### 1. 赚钱进度条（hero 卡内）
- 语义：**已赚 ÷ 全天应赚**。全天应赚 = `hourly_rate × daily_hours`，
  两个字段都是 `DayStatus` 现成返回值 → **零 Rust 改动**
- 形态：6px 金色渐变填充条（width 过渡 0.6s，金额每秒跳变时平滑爬升）
  + 说明行「全天应赚 ¥xxx · 62%」
- 边界：休息日或时薪为 0 → 整块隐藏；已下班 earned 封顶 100%（加班费不进此条）
- 更新点：`tick()` 内基于已有 `s` 计算，不加 IPC

### 2. hero 化
- `.live` 卡：内边距加大、`.big` 38→46px、h2 居中、网格数字 15→16px，
  底色比普通卡微亮一档——同屏其他卡自动退为次要层级
- 监控三合一：今日活动/应用使用/媒体播放三张卡合并为一张「今日监控」卡：
  - 头部 = 标题 + 内联小分段（活动/应用/媒体，金色药丸高亮）+ 「查看明细 ›」
  - 「查看明细」跟随当前小分段跳 viewAct/viewApp/viewAudio
  - 三个预览面板 DOM 原样保留（act_left 等统计 id、appuHomeList、audioHomeList
    均由现有 paint 函数维护）→ 渲染逻辑零改动
  - `.mon-body` 设 min-height 免得切分段时卡片跳动
- 加班总览卡不动

### 3. 切换动效
- `@keyframes viewIn`（opacity 0→1 + translateY 4px→0，150ms ease-out）挂在 `.app` 上，
  display:none→可见时自动重放，无需 JS
- `prefers-reduced-motion: reduce` 时关闭（原生应用的基本素养）

## 涉及文件

| 文件 | 改动 |
|---|---|
| frontend/index.html | live 卡加进度条；三监控卡合并为 monitorCard |
| frontend/styles.css | viewIn 动画；hero 卡样式；进度条样式；mon-head/mon-seg 样式 |
| frontend/app.js | tick() 加进度计算；监控小分段绑定 + monDetailBtn；删 3 个旧明细按钮绑定 |
| scripts/test_hero.js | 新增（进度条契约、监控映射、动画存在、$() 兜底） |
| CHANGELOG.md | 未发布区补记录 |

## 不做的事

- 不动 Rust（数据字段够用）
- 不动加班总览卡、明细页、设置页
- 不引入 CSS 变量重构（金色字面量仍散落各处，另行专题）

## 测试

- `node scripts/test_hero.js`：进度条元素与计算守卫、监控映射完整、旧按钮 id 无残留、
  动画与 reduced-motion 存在、`$()` 全量兜底
- 其余 scripts/*.js 全量回归 + `build.bat debug` 编译验证
