# Pull request and CI snapshot, 18 October 2026

This page records pull request and first-run CI counts for
`traverse-framework/traverse` as of 18 October 2026. The figures below are
computed from
[`2026-10-18/data/first_runs.json`](2026-10-18/data/first_runs.json)
by
[`2026-10-18/scripts/stats.py`](2026-10-18/scripts/stats.py).
The file SHA-256 is
`f78ba8cf39f3e5a6e920ce5657d57f53a678e4656cf91f93e98858f9e699a67b`
(196,115 bytes). The full GraphQL pull is not in the repository. This reduced
file is what the counters read.

## Figures

949 pull requests in total. 854 (90 percent) carry an agent marker. 54 are
bot. 41 are unknown. The 41 unknown are not confirmed human. 854 divided by
949, rounded to the nearest percent, is 90 percent.

Agent-generated pull requests merged from 18 September 2026 00:00 Mountain
Daylight Time through this 18 October 2026 snapshot (actual merges 18 September
to 15 October): 133, of which 106 (79.7 percent) passed their first CI run. Of
the 27 failures, 24 were code, test or build failures and 3 were metadata only.

Agent-generated pull requests merged from 19 August 2026 through 18 October
2026 (the 60 days to 18 October): 306, of which 221 (72.2 percent) passed
their first CI run.

The 60 day denominator is 306. One of those 306 still had a pending first
rollup, so it is counted in the 306 and not in the 221. The other 84 failed.
221 divided by 306, printed to one decimal place, is 72.2 percent. The same
rounding prints 106 divided by 133 as 79.7 percent.

## Snapshot date and collection

The snapshot date is 18 October 2026. Collection used read-only `gh` calls
against `traverse-framework/traverse`. A pull request listing was taken that
afternoon, Mountain Daylight Time. The first-run file on this page was built
from a pull the same day, at about 15:49 Mountain Daylight Time. That pull
still contained 949 pull requests, equal to the GraphQL `totalCount`. No pull
request had more than 100 commits, so no commit list was truncated. The newest
pull request was created at 16 October 2026 00:34:40 Mountain Daylight Time.
The latest `mergedAt` in the file is 15 October 2026 21:30:30 Mountain
Daylight Time. No pull request in this
snapshot merged on 16, 17, or 18 October.

API timestamps are UTC. The scripts convert them with a fixed offset of six
hours behind UTC. They do not use a timezone database. On these dates that
offset is Mountain Daylight Time (UTC-6).

Pull requests were read with this GraphQL query, 40 per page, ordered by
`CREATED_AT` ascending, following `pageInfo.endCursor` until `hasNextPage` was
false. On the first page the `$c` variable is omitted. After that it is the
end cursor. The text below is the query string in
`2026-10-18/scripts/fetch_pull_requests.py`, including the line break before
`commits`.

```text
query($c:String){repository(owner:"traverse-framework",name:"traverse"){pullRequests(first:40,after:$c,orderBy:{field:CREATED_AT,direction:ASC}){totalCount pageInfo{hasNextPage endCursor} nodes{number title state createdAt mergedAt closedAt headRefName baseRefName isDraft additions deletions author{login __typename} mergedBy{login} labels(first:20){nodes{name}} body
 commits(first:100){totalCount nodes{commit{oid committedDate message authors(first:5){nodes{name email user{login}}} statusCheckRollup{state} checkSuites(first:20){nodes{conclusion status app{slug} workflowRun{workflow{name} event}}}}}}}}}}
```

For each pull request, the first CI run is the first commit in that list whose
`statusCheckRollup` is present. Its state is stored as `roll`, and its SHA as
`sha`.

Failing check names, for a first-checked commit whose rollup state was
`FAILURE` or `ERROR`, come from saved REST responses:

- `GET /repos/traverse-framework/traverse/commits/{sha}/check-runs`
- `GET /repos/traverse-framework/traverse/commits/{sha}/status`

Those response bodies are not in this snapshot. `build_first_runs.py` reads
them from a directory you pass in and writes a new `first_runs.json` only
under the output directory you pass in. A check run is kept when `conclusion`
is `failure`, `timed_out`, `startup_failure`, or `action_required`. A commit
status is kept when `state` is `failure` or `error`, and stored as `status:`
plus the context. The script stops if `total_count` on the check-runs body is
greater than 100. It does not request a further page. The names already stored
on each row are the `fails` arrays in the committed file.

`fetch_check_runs.py` repeats a separate GraphQL check-suite query (suites in
batches of 25, 15 suites and 60 check runs per commit). The figures on this
page do not read its output.

## Agent-generated

The scripts label a pull request `bot`, `ai`, or `unknown`. On this page,
agent-generated means `ai`.

`bot` is tested first. The pull request is `bot` when `author.__typename` is
`Bot`. A bot pull request is not also counted as agent-generated. This
snapshot has 54.

A non-bot pull request is agent-generated when at least one marker below is
present. One marker is enough. A pull request can carry more than one.

- The head branch contains `/`, and the text before the first `/`, compared
  in lower case, is `codex`, `claude`, `cursor`, `copilot`, or `devin`.
- A label name, compared in lower case, starts with `agent:`.
- The body matches, case insensitive: `generated with claude code`,
  `generated with [claude code`, `generated with cursor`, `cursor agent`, or
  `generated by` followed by `codex`, `copilot`, or `devin`. A bare occurrence
  of the word `codex` in the body is not a marker.
- A commit message matches a `Co-authored-by:` trailer whose name starts with
  `claude`, `cursor`, `copilot`, `codex`, `devin`, `openai`, `chatgpt`, or
  `gemini`. The match is case insensitive.
- A commit author name or email matches `noreply@anthropic.com` or
  `cursoragent@cursor.com`, or contains `copilot`, `devin`, or `codex`. The
  match is case insensitive and applies to the name and the email.

Anything else is `unknown`. Unknown means no marker was present. It does not
mean the pull request was confirmed to be written by a person. This snapshot
has 41 unknown pull requests, and none of them are recorded as confirmed human
authorship.

`classify.py` applies this rule to a pull request dump. The committed
`first_runs.json` already stores the resulting `cls` for each of the 949 pull
requests. `stats.py` counts that field. It does not re-read pull request
bodies.

## First CI run

Pass means rollup state `SUCCESS`. Fail means `FAILURE` or `ERROR`. Pending
means `PENDING`. A pending rollup stays in the window denominator and is not a
pass. Two pull requests in the whole snapshot have no rollup. Neither is in
the merged windows below.

The rollup is the latest attempt GitHub reports for that commit. A later
manual rerun that succeeded counts as a pass. Commits removed by a force-push
are not in the list the pull returned.

## Windows

A pull request is in a window when `mergedAt` is present and, after conversion
to Mountain Daylight Time, is greater than or equal to the window start. The
scripts apply no separate end cutoff. The snapshot date supplies the end:
this file has no merge after 15 October 2026 21:30:30 Mountain Daylight Time.

- 18 September 2026 00:00 Mountain Daylight Time through the 18 October 2026
  snapshot. Agent-generated `mergedAt` values in this set run from 18 September
  2026 11:08:35 Mountain Daylight Time to 15 October 2026 21:30:30 Mountain
  Daylight Time. That is the "actual merges 18 September to 15 October" range.
  Count: 133. Passed: 106.
- 19 August 2026 00:00 Mountain Daylight Time through the 18 October 2026
  snapshot. This is the 60 days to 18 October. Agent-generated `mergedAt`
  values in this set run from 20 August 2026 08:52:45 Mountain Daylight Time to
  15 October 2026 21:30:30 Mountain Daylight Time. No agent-generated pull
  request in the snapshot merged on 19 August. Count: 306, including 1 pending.
  Passed: 221.

## Failure kind

Used for the 27 failures in the 18 September window. Metadata check names are
`spec-alignment`, `pr-hygiene`, `cla / cla`, `version-guard`, and
`baseline / governance-baseline`.

- Metadata only: every failing check is in that set. In the 18 September
  window this is 3 agent-generated pull requests (1 whose only failure was
  `spec-alignment`, and 2 whose failures were other metadata checks).
- Code, test, or build: at least one failing check is outside that set, or
  the rollup failed and the failing-check list is empty. In the 18 September
  window this is 24 agent-generated pull requests. Pull request 1664 is in
  this group: its rollup failed and the saved failing-check list is empty.
  The only recorded job outcome for that commit was a cancelled
  `native-artifact-certification` run, which the failure filter does not list.

## How to rerun

From any working directory, this reads the committed snapshot and writes
nothing:

```text
python3 docs/metrics/2026-10-18/scripts/stats.py
```

The path is relative to the repository. The script locates
`docs/metrics/2026-10-18/data/first_runs.json` from its own file path, not from
the current directory. The lines that start with `HEADLINE` are the figures on
this page:

```text
HEADLINE total=949 agent=854 bot=54 unknown=41 agent_share=90.0%
HEADLINE 30d agent=133 pass=106 rate=79.7% fail=27 code=24 metadata=3 pending=0
HEADLINE 60d agent=306 pass=221 rate=72.2% fail=84 code=50 metadata=34 pending=1
```

`30d` is the 18 September window. `60d` is the 19 August window. `agent_share`
is 854/949 printed to one decimal place.

Optional commands, still with no network access:

```text
python3 docs/metrics/2026-10-18/scripts/weekly.py
python3 docs/metrics/2026-10-18/scripts/stats.py --output /path/to/output
```

`weekly.py` prints Monday-start tables and writes nothing. `--output` writes
`first_runs_cat.json` only inside that directory.

To apply the marker rule to a pull request dump, or to repeat the `gh` reads,
pass an output directory. These commands do not modify the committed snapshot
unless you point `--output` at it.

```text
python3 docs/metrics/2026-10-18/scripts/classify.py --input prs.json --output /path/to/output
python3 docs/metrics/2026-10-18/scripts/fetch_pull_requests.py --output /path/to/output
python3 docs/metrics/2026-10-18/scripts/fetch_check_runs.py --input /path/to/output/prs.json --output /path/to/output
python3 docs/metrics/2026-10-18/scripts/build_first_runs.py --input prs.json --check-runs-dir /path/to/check-runs --output /path/to/output
```

`fetch_pull_requests.py` and `fetch_check_runs.py` call `gh` and need
permission to read this repository. `build_first_runs.py` needs the saved REST
bodies named `{sha}.json` and `{sha}.status.json`. The standard library is
enough for every script. No third-party package is required.
