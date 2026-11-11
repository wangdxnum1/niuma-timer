# Monitor history navigation for 1.11.5

**Goal:** Make existing keyboard/mouse, application and media history easier to browse and release a patch version.

**Design:** Keep previous/next day buttons and add a native date picker plus return-to-today button. Validate calendar dates and reject future input. Choosing today follows midnight rollover. Each monitor keeps one current request token; duplicate polls for its pending date are coalesced and stale success/error results cannot alter current data. A selected historical date never paints data from another date. Date changes hide old details behind loading feedback, failures offer retry, and disabled monitoring has an explicit explanation. Existing queries, collection and salary semantics are preserved.

**Constraints:** Preserve wheel paging and keyboard behavior. No backend, schema or dependency changes. Date controls fit the existing narrow window and are independently labelled. Historical headings must not claim their values are today's values.

- [x] Add regression tests for date selection, invalid/future dates, today reset, stale responses, duplicate polls and failure retry.
- [x] Implement navigation, scoped request handling, loading states and accurate headings; update docs.
- [x] Run tests, inspect layout and obtain a fresh review.
- [ ] Run the existing release pipeline for 1.11.5 and verify cloud success, release assets and updater manifest.

Implementation verification: 42 frontend scripts passed. Independent review approved the request/date/cache guards and separately tested midnight rollover; the rollover regression is now permanent. A local preview using actual markup, CSS and render functions confirmed the 400px layout and date/title/button synchronization after returning to today. Existing wheel paging and keyboard source are unchanged.
