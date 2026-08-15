# initial-case

Three-arm scripted first-match (`chanvoy_wait`, `sms_inbound`, `job_complete`).
Registrations start from `baseline_policy=latest`. Bind resolves an exclusive
provider cursor; the policy label is not a cursor.

`waitprims wait --registration-set registration_set.json --request live_wait_request.json --script live.json`
