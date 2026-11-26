# Sourced by the scripts that run inside a container.
start() { # start <command...>: the server, in the background; SUP is the process that watches
  "$@" > /tmp/out.log 2> /tmp/err.log &
  SUP=$!
  for i in $(seq 1 100); do
    if curl -s -m 1 -o /dev/null http://127.0.0.1:8080/ping; then break; fi
    sleep 0.1
  done
}
workers() { pgrep -P $SUP | tr '\n' ' '; }
stat() { echo "pid=$1 fds=$(ls /proc/$1/fd 2>/dev/null | wc -l) threads=$(ls /proc/$1/task 2>/dev/null | wc -l) rss=$(awk '/VmRSS/{print $2}' /proc/$1/status 2>/dev/null)kB"; }
