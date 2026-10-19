#!/bin/sh
set -eu

log=${1:?usage: sh tools/actions/test-summary.sh TEST_LOG}

awk '
function open_suite(name) {
    if (pending) row(suite, "| | | | did not finish |")
    suite = name
    pending = 1
}
function row(name, cells) {
    rows[++count] = sprintf("| `%s` %s", name, cells)
}
{ gsub(/\033\[[0-9;]*m/, "") }
/^ *Running / {
    name = $0
    sub(/^ *Running /, "", name)
    sub(/ \(.*\)$/, "", name)
    open_suite(name)
    next
}
/^ *Doc-tests / {
    name = $0
    sub(/^ *Doc-tests /, "doc-tests ", name)
    open_suite(name)
    next
}
/^test .* \.\.\. FAILED$/ {
    name = $0
    sub(/^test /, "", name)
    sub(/ \.\.\. FAILED$/, "", name)
    failures[++failed_tests] = name
    next
}
/^test result: / {
    row(suite, sprintf("| %d | %d | %d | %s |", $4, $6, $8, $NF))
    pending = 0
    passed += $4
    failed += $6
    ignored += $8
    seconds = $NF
    sub(/s$/, "", seconds)
    elapsed += seconds
}
END {
    if (pending) row(suite, "| | | | did not finish |")
    print "### Tests"
    print ""
    if (!count) {
        print "No test binary reported results."
        exit
    }
    print "| Suite | Passed | Failed | Ignored | Time |"
    print "|---|--:|--:|--:|--:|"
    for (i = 1; i <= count; i++) print rows[i]
    printf "| **Total** | **%d** | **%d** | **%d** | **%.2fs** |\n", passed, failed, ignored, elapsed
    if (failed_tests) {
        print ""
        print "Failed:"
        print ""
        for (i = 1; i <= failed_tests; i++) printf "- `%s`\n", failures[i]
    }
}
' "$log"
