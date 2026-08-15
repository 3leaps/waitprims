# initial-case

Three-arm scripted first-match and one poll-cycle (`chanvoy_wait`,
`sms_inbound`, `job_complete`). Registrations start from
`baseline_policy=latest`. Bind resolves an exclusive provider cursor; the
policy label is not a cursor.

```
waitprims wait --registration-set registration_set.json --request live_wait_request.json --script live.json
waitprims poll --registration-set registration_set.json --request poll_cycle_request.json --script poll.json
```
