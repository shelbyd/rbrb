#! /bin/bash

set -euxo pipefail

function clean_up {
  kill $proc_1
  kill $proc_2
  exit
}

trap clean_up SIGHUP SIGINT SIGTERM

RUST_LOG=debug cargo run --example boilerplate -- --local-port 7000 --local-index 0 127.0.0.1:7001 &
proc_1=$!

cargo run --example boilerplate -- --local-port 7001 --local-index 1 127.0.0.1:7000 > /dev/null 2>&1 &
proc_2=$!

wait $proc_1
wait $proc_2
