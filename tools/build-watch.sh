#!/bin/sh
# Build helper: rebuilds whenever a `.build-request` file appears in the project root
# and writes the full output to `build.log`. Lets someone with access to the folder
# (but not to your terminal) trigger builds and read the errors.
#
#   sh tools/build-watch.sh        (Ctrl+C to stop)
#
# A request file containing "test" runs `cargo test --release` before building.
cd "$(dirname "$0")/.." || exit 1
echo "Watching $(pwd) for build requests. Ctrl+C to stop."
while true; do
  if [ -f .build-request ]; then
    request=$(cat .build-request)
    rm -f .build-request
    echo "[$(date +%T)] building…"
    {
      echo "started: $(date)"
      status=0
      case "$request" in
        *test*) cargo test --release --color never 2>&1 || status=$? ;;
      esac
      if [ "$status" -eq 0 ]; then
        cargo build --release --color never 2>&1 || status=$?
      fi
      echo "exit code: $status"
    } > build.log.tmp
    mv build.log.tmp build.log
    echo "[$(date +%T)] $(tail -n 1 build.log)"
  fi
  sleep 1
done
