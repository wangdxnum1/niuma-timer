# Working-time slacking implementation

**Spec:** ../specs/2026-11-12-worktime-slack-design.md

1. Collector/data module task: implement work_usage, minute capture, table migration/lifecycle registration and AppUsageSummary.work_usage. Own work_usage.rs, app_usage.rs, db.rs, maintain.rs and main.rs module declaration. Test real interval logic and in-memory SQLite; preserve raw totals and transaction behavior.
2. Bill/UI task: integrate shared range calculation into weekbill; preserve raw attendance/salary conditions; expose legacy metadata. Update homepage, reports/CSV/images and tests with explicit known/unknown/partial wording. No fallback to the old all-day formula.
3. Run full project gates, fresh spec/code review and targeted regression checks. Update bilingual docs/release materials. Build a reviewable artifact before any publishing action.

Implementation is performed in the existing authorized workspace. Do not commit during implementation. User approved the new-record/legacy policy; retain all legacy raw records and keep wheel behavior unchanged.

Validation (2026-11-12): implementation tasks 1 and 2 completed. Independent data/UI reviews resolved the fixture field and trend coverage findings. `build.bat test` passed (288 Rust tests, 43 Node scripts, Python suites 29/11/4; fmt/clippy/dependency audit included). New UI regression fails against the previous all-day implementation and passes with work-window statistics. A local fixture at the default 520px width verified mixed/legacy-only homepage states; no user data was used. Bilingual README and 1.11.6 changelog prepared. The standard release pipeline repeats gates at version 1.11.6 before signing, committing and pushing.
