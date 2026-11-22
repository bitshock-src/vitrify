# fails until it has run more than FLAKY_FAILS times (default 1); the
# counter lives in $FLAKY_DIR, outside every checkout
dir=${FLAKY_DIR:?FLAKY_DIR names a directory outside the checkout}
mkdir -p "$dir"
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
test "$n" -gt "${FLAKY_FAILS:-1}"
