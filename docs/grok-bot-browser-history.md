# Experimental Grok Bot browser history

`grok-bot` collects separate, owner-bound **reported billing records** through an existing local browser session. It is not a token-report client or a replacement for Cursor sync. These snapshots never enter reports, submissions, Wrapped or leaderboards. Subscription quota support is separate from this history command.

## Requirements and commands

This transport is experimental **macOS support requiring an already installed, running, logged-in local Aside Browser/CLI and Python 3**. It uses the supported `aside repl --host local` interface. Tokscale does not install/update Aside, log in, export cookies, read native credentials, refresh/exchange tokens or change browser/security settings. Windows runtime and unattended native API authentication are not supported by this command.

```sh
# Use the exact default billing range requested by the actual dashboard UI.
tokscale --no-spinner grok-bot sync --transport aside

# A private local view; --json prints the reported records and owner/range.
tokscale --no-spinner grok-bot history
tokscale --no-spinner grok-bot history --json

# Automated checks: no owners, IDs, dates, paths, record/token totals or costs.
tokscale --no-spinner grok-bot sync --transport aside --redacted
tokscale --no-spinner --offline grok-bot history --redacted
```

An explicit interval uses `--start <RFC3339-UTC-Z>` and `--end <RFC3339-UTC-Z>` together. Start is inclusive and end exclusive. The original validated strings are retained; no weekly quota dates, native billing period, Unix-zero start, all-time retention or point/day timezone is substituted. Without those options the range comes from an actual initial dashboard billing URL in same-origin resource timing after normal UI navigation. GET method and user/page fields are verified separately; timing is not used to infer a method or body. Ambiguous/missing initial URLs fail rather than guessing a calendar range.

`--max-pages` defaults to 128 (2–512) and includes owner discovery and the final owner check. `--timeout` defaults to 300 seconds (10–600). The controller issues separate, short supported REPL commands for progress and transfer; it does not change the browser evaluation or REPL command time limits. Each API request is started once, with a deadline of at most 60 seconds and never beyond the overall deadline. Short browser evaluations check its pending promise; they do not resend the request. Discovery and each fetched page yield back to the local REPL before another request. A private async iterator retains the fixed owner/range and validation state inside the owned page. The complete walk is not one long browser evaluation. Individual reads are bounded, with a 256 KiB page limit, 8 MiB total response budget and 16 MiB private transfer limit. Limits, auth errors,429, invalid/HTML responses, timeouts and incomplete walks fail without automatic retry or changing existing snapshots. The UI's own normal dashboard background requests are not blocked or altered.

## Ownership, pagination and meaning

The initial actual user-scope billing request binds the browser principal from Bot records. Leading pages without Bot may be inspected under the same bounds to find that owner; an unavailable/wholly empty owner cannot be manufactured. The requested walk fixes scope, range and every noncursor field, reads pages sequentially and uses exactly the returned nextCursor. Other-product and empty-Bot pages do not terminate it. All item IDs are checked for duplicates before exact `product=grok-bot` records are selected. Bot principal type/id must match, and the same observed owner anchor is re-read after the terminal page. Account/scope drift, malformed/missing cursor, cycles, duplicate/conflicting IDs, out-of-range events or a failed final owner check abort the result.

The browser source is `https://grok.com/dashboard/api/dashboard/v2/query-billing-items`, GET, personal `scope=user`, a dashboard-observed `pageSize=20` and collector `pageSize=100`, verified with two full consecutive real responses. There is no verified server product filter; the Product control groups chart dimensions. Tokscale does not send an invented product parameter or the native app's `products=[sand]`. Native app/Cursor tokens are never sent to Grok.

`API pagination complete` means only that the requested cursor walk reached a consistent terminal response. Retention, corrections/deletions and an immutable multi-page snapshot are unverified. A successful owner-bound empty range means **no records returned**, not zero historical usage. It cannot replace an earlier nonempty snapshot for the same owner/range. Older owner/range versions are retained rather than merged or deleted.

Records preserve `billingItemId`, `eventTime`, product, principal, category/type, `hasTokenMeters`, these exact reported usage keys and `totalTokens`:

- `prompt-text-tokens`
- `completion-text-tokens`
- `cached-prompt-text-tokens`
- `cache-write-text-tokens`

Meter values remain unsigned decimal **strings**, including values beyond JavaScript's exact Number range. Each value and checked sum must fit unsigned 128-bit arithmetic; missing, malformed, fractional, signed or overflowing values fail. All four must be present and their exact sum must equal the reported total. Unknown Bot meter schemas are unsupported; missing meters are never zero-filled. Other products' token/model/cost schemas are not normalized into Bot.

`model` and `cost` are always explicit JSON null. There are no inferred input/output/cache buckets, models, money scales, sessions or event IDs. Canonical `UnifiedMessage` conversion is intentionally absent. The activity view identifies source, requested range, completion and coverage limits; it displays a cached owner at collection time, not the current browser account.

## Private storage and process boundary

Default storage is `<Tokscale cache>/grok-bot-browser/history.json`; `TOKSCALE_GROK_BOT_DATA_DIR` can select a dedicated absolute local directory. Storage inside a Git worktree, symlink paths and nonprivate ownership/modes are rejected. Directories are 0700 and files 0600. Do not use a shared/export directory. This cache is separate from Cursor and every scanner/client registry.

Only the existing Aside session's already allowed root holds transient 0600 FIFOs. A control FIFO carries collection options and a framed data FIFO carries minimal Bot records from the local REPL to Rust; neither is an ordinary data export file. Root/type/owner/mode/device/inode/no-symlink, size, EOF and deadline checks protect that channel. All startup banners, raw child stdout/stderr and errors stay private or are discarded, including with `--debug`. No source records are printed in REPL console or passed as process arguments/code strings.

Rust independently validates the received source snapshot before locking a single private cache transaction, writing a 0600 staging file in that same private directory and atomically renaming it. No independently updated account index exists. Failures preserve prior cache bytes. Successful owner/range versions coexist, and no raw account identifier or identifier hash appears in a filename. `--redacted` and local readback expose only fixed metadata/booleans; normal `history --json` is an explicit private-data display, not an upload format.

The command does not initialize scanner timezone settings or access other account configuration. The owned browser tab, private iterator/payload, FIFOs and local REPL are cleaned up on completion/error. An OS/tool/login requirement is reported rather than bypassed; there is no token/cookie setup flow.

## Development checks

```sh
node crates/tokscale-cli/src/commands/grok_bot_history/collector.test.cjs
cargo test --locked -p tokscale-cli --bin tokscale commands::grok_bot_history:: --all-features
cargo test --locked -p tokscale-cli --test grok_bot_history --all-features
```

Normal tests use synthetic data only. The explicitly ignored `local_aside_reverse_fifo_synthetic`, `local_aside_browser_runtime_synthetic` and `local_aside_collector_pipeline_synthetic` and `local_aside_collector_delayed_synthetic` tests exercise the real local REPL, `about:blank`, runtime features and the full synthetic collector/FIFO path, including delayed pages whose aggregate exceeds the observed single-evaluation timeout; they make no Grok/native request. These optional tests require the runtime prerequisites above and are never run by the normal suite.


## Safe failure diagnostics

Collector errors may append only `http_status`, `phase`, `successful_pages` and `elapsed_ms` to the fixed error code. Status 401 is `http_auth`, 403 is `http_permission`, 429 is `http_rate_limit`, and 5xx is `http_server` (potentially transient). There is no automatic retry, credential fallback or limit increase. A missing HTTP response is represented by `http_status=none`.

`successful_pages` counts distinct responses that passed JSON/page validation, including owner discovery and verification; it is not a record or token total. `elapsed_ms` is the browser collector's relative elapsed time. Phases are limited to `discover`, `owner`, `walk`, and `recheck`. The JS producer, Python relay and Rust consumer validate the same four-field shape and numeric bounds. Malformed, missing, extra or unrecognized fields are discarded without printing their contents. URLs, queries, cursors, identities, response bodies and credentials are excluded.

Pure synthetic validation includes `node crates/tokscale-cli/src/commands/grok_bot_history/collector.test.cjs`, `python3 -B crates/tokscale-cli/src/commands/grok_bot_history/aside.test.py`, and the normal related Rust tests. The latter cover safe control-channel errors and preservation of both absent and existing caches. These commands do not run the ignored account diagnostics.

## Bounded local day-by-day backfill

The optional Node.js helper `scripts/grok-bot-backfill.cjs` orchestrates the existing command; it does not provide another authentication transport. It requires an existing valid private snapshot to pin the account, and the built experimental CLI plus the same local Aside/Python prerequisites. It accepts explicit UTC midnight bounds, processes adjacent half-open days newest first, and skips only completed, owner-checked snapshots for the same account and exact interval. Those immutable snapshots are the checkpoint; no cursor or credential is exported into another manifest.

```sh
node scripts/grok-bot-backfill.cjs \
  --binary /absolute/path/to/tokscale \
  --data-dir /absolute/private/cache/directory \
  --start '<inclusive-UTC-midnight-ending-in-Z>' \
  --end '<exclusive-UTC-midnight-ending-in-Z>' \
  --max-days 7
node --test scripts/grok-bot-backfill.test.cjs
```

One invocation is limited to seven requested days by default, with an explicit maximum of 31. Each uncached day retains the CLI defaults of 100 rows per page, 128 requests/pages and 300 seconds. This is a request-plan bound, not an assertion about API retention. The helper stops at the first failure, does not retry auth/permission/rate-limit/server failures, and checks cache byte preservation on a failed collection. Successful days are independently stored and read back before the next day. A later invocation resumes by inspecting those completed snapshots. Never automatically rerun a failed range without diagnosing its reported failure.

After a new collection the helper compares the returned owner to the pinned cached owner, validates the requested interval and record IDs/timestamps, and reads back through the CLI. A changed owner stops the batch; independently owner-bound versions are never merged. Skipping cached days alone does not authenticate the currently selected browser account. Output contains only operational status, day/record counts and sanitized failure diagnostics; records, identities, timestamps, meter values and credentials stay private.

Successful adjacent days establish completion only for those requested intervals. Empty earlier dates do not establish account creation, the earliest retained event, or complete historical coverage. Retention and concurrent correction/deletion semantics still require source evidence. This helper does not turn browser snapshots into Cursor records, canonical reports or leaderboard submissions.
