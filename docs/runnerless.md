# Codex review enforcement

`.runnerless-ci.ts` uses Runnerless's versioned SDK to evaluate Codex review
evidence for open pull requests. PR, review, comment, thread and check events
refresh `Runnerless / Codex review`. A successful check requires the host's review
policy to pass; pending or unsuccessful review evidence does not pass.

Runnerless loads this program from the PR's trusted **base** revision. Adding it
in the first implementation PR therefore enables the program after that PR is
merged. Repository enrollment and GitHub App access are also required; a file in
Git alone does not activate the service.

For the empty-base bootstrap PR, require the App-owned `codex/review-gate` check
when the existing review service reports it. After this program is on `main`,
verify a live `Runnerless / Codex review` check and require it through a GitHub
ruleset pinned to the reporting App. Preserve existing required checks, including
Rust CI. Do not require a check that the service cannot yet publish.

Request a review for the current head with a PR comment containing exactly
`@codex review`. After fixing findings and pushing a new head, request another
review. Resolve findings only after verifying the claim and publishing any fix.
The Rust build, tests, zero-allocation benchmark and RISC-V RAM check run separately
in `.github/workflows/ci.yml`.
