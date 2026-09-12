# 导航改版：顶部三 tab + 明细分段直达

- 日期：2026-09-12
- 状态：已与用户确认（顶导方案 + 四页合一）
- 背景：二级页导航从「‹ 返回」钻入式改为左侧图标栏（未提交的 WIP）后，用户反馈 6 项 tab 偏多、窗口因侧栏 440→488 变宽。

## 目标

1. 窗口宽度回到 440×600（去掉 48px 侧栏）
2. 常驻导航只留 3 项：主页｜明细｜设置
3. 保留「任意明细直达」能力：主界面卡片按钮与分段子导航均直达

## 方案

### 顶部导航条（topnav）
- 窗口顶部一条 3 文字 tab：主页｜明细｜设置；当前项金色药丸高亮，悬停浅白
- 「设置」右侧分离（原生惯例）；「主页」首位
- 「明细」tab 是聚合 tab，对应 4 个二级页，点击进入**上次浏览的**明细视图（默认加班）

### 明细分段条（segbar）
- 顶导下方一条分段控件：加班｜活动｜应用｜媒体，仅在 4 个明细视图可见时显示
- 选中分段金色高亮；每个分段保留自己的标题与导航（加班=月份，其余=日期）

### 实现策略（契约不动）
- 4 个明细视图 DOM 原样保留（viewOt/viewAct/viewApp/viewAudio），`curView` 语义不变
  → 懒渲染守卫（`curView !== "viewAct"` 等）、返回主页日期复位（`resetHistDates`）、
  设置页自动保存（`saveIfChanged`）零改动
- 只改导航层：
  - `showView` 末尾的 rail 高亮 → topnav 高亮映射（4 个明细视图都映射到「明细」tab）
    + segbar 显隐/分段高亮 + `lastDetailView` 记忆
  - 事件绑定：`.topnav-item`（detail → lastDetailView）与 `.seg-item` → `showView`
- 主界面 4 张卡片「查看明细 ›」按钮保留，直达对应分段（自动归入「明细」tab）

## 涉及文件

| 文件 | 改动 |
|---|---|
| frontend/index.html | rail → topnav + segbar |
| frontend/styles.css | body 改纵向 flex；删 rail 样式；加 topnav/segbar 样式；`.app` 高度改 flex 撑满 |
| frontend/app.js | showView 高亮映射 + lastDetailView；绑定改造；删 rail 绑定 |
| src-tauri/tauri.conf.json | width 488 → 440 |
| scripts/test_nav_rail.js | 删除，改写为 scripts/test_topnav.js |
| CHANGELOG.md | 未发布区补导航改版记录 |

## 测试

- `node scripts/test_topnav.js`：topnav 3 tab 顺序（主页首位/设置末位）、segbar 4 段齐全、
  app.js 无 rail/back 残留引用、`$()` 引用兜底
- 其余 scripts/*.js 全量回归
- `build.bat debug` 编译验证（build.rs 自动同步前端缓存戳）
