# waitprims-fs

Native local-filesystem wait arm for `waitprims`.

```toml
waitprims-fs = "0.2"
```

This crate implements the existing `waitprims_async::Observer` seam with
`notify`'s native `RecommendedWatcher`. It is not a daemon, a shell-style
watch command, a durable event ledger, or a polling fallback.

The observer:

- accepts only `method_id = "file_watch"`;
- addresses paths relative to a caller-configured canonical root;
- accepts the closed predicate set `pred:file-create`, `pred:file-write`,
  `pred:file-remove`, `pred:file-rename`, and `pred:file-any`;
- supports only `baseline_policy = latest`;
- keeps same-bind custody for `restore_ready`, but makes no cross-bind replay
  claim;
- returns typed fail-closed observations for overflow, native watcher failure,
  cursor uncertainty, ambiguous classification, and unsupported filesystem
  posture.

Network and unknown filesystem postures are unsupported in this release.
Detection is caller-declared and therefore limited. There is no
`PollWatcher` or `NullWatcher` fallback.

The event-reference sink receives only a normalized event class and
root-relative slash-separated paths. File contents, absolute host paths,
credentials, and capability values are never descriptor fields.
