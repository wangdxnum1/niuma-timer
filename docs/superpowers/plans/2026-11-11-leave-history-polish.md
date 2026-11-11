# Leave history and report readability

**Goal:** Improve existing leave-history navigation and report readability, then release 1.11.4 as requested.

**Design:** Query leave records for a selected calendar month, with previous/next/current-month buttons and direct month selection. Saving a leave entry selects its month. Only the newest query may paint the list; loading and errors remain visible and failed queries can be retried. Existing leave/calculation semantics stay unchanged. Reports show leave counts with dates in a native collapsed disclosure. Milestone copy explains retained-data aggregation and labels first_date specifically as the earliest keyboard/mouse record.

**Constraints:** Preserve wheel paging. No database/schema, salary, aggregation or updater changes. Keep new UI consistent with existing controls and responsive within the fixed-width window. Source-backed regression tests must cover stale responses, leap-year boundaries, historical save visibility, request failures and report empty/long states.

- [x] Implement and verify monthly leave browsing and historical-save visibility.
- [x] Implement and verify compact leave report details and milestone explanation.
- [x] Update bilingual docs and release notes; run frontend regression and independent review.
- [ ] Run standard project gates, build and release 1.11.4; verify cloud success, assets and updater manifest.

Implementation verification: 41 frontend scripts passed. Fresh read-only review found no blocking issues. A local preview using the actual markup, CSS and render functions confirmed the month controls and expanded report dates at a 400px content width. Wheel and keyboard source are unchanged from 1.11.3. Formal publication is performed by the existing release script and CI.
