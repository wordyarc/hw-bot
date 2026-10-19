#!/bin/sh
set -eu

summary=${1:?usage: sh tools/actions/coverage-summary.sh SUMMARY_JSON}

jq -r --arg root "$(pwd)/" '
    .data[0]
    | (.files
        | map({
            module: (.filename | ltrimstr($root) | split("/") | .[0:2] | join("/") | sub("\\.rs$"; "")),
            summary
        })
        | group_by(.module)
        | .[]
        | [.[0].module] + (map(.summary) | [
            (map(.lines.covered) | add), (map(.lines.count) | add),
            (map(.functions.covered) | add), (map(.functions.count) | add),
            (map(.regions.covered) | add), (map(.regions.count) | add)
        ])),
      (.totals | [
        "total",
        .lines.covered, .lines.count,
        .functions.covered, .functions.count,
        .regions.covered, .regions.count
      ])
    | @tsv
' "$summary" | awk -F '\t' '
function percent(covered, total) {
    return total ? sprintf("%.1f%%", 100 * covered / total) : "-"
}
function bar(covered, total,   filled, text, i) {
    filled = total ? int(10 * covered / total) : 0
    text = ""
    for (i = 0; i < 10; i++) text = text (i < filled ? "█" : "░")
    return text
}
BEGIN {
    print "### Coverage"
    print ""
    print "| Module | Lines | Functions | Regions |"
    print "|---|---|--:|--:|"
}
$1 == "total" {
    total = sprintf("| **Total** | `%s` **%s** (%d / %d) | **%s** | **%s** |", \
        bar($2, $3), percent($2, $3), $2, $3, percent($4, $5), percent($6, $7))
    next
}
{
    module = $1
    sub(/^src\//, "", module)
    printf "| `%s` | `%s` %s (%d / %d) | %s | %s |\n", \
        module, bar($2, $3), percent($2, $3), $2, $3, percent($4, $5), percent($6, $7)
}
END { print total }
'
