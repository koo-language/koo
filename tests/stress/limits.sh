#!/bin/sh
# A server under a limit on its processes, as a container gives it: the process that
# watches the workers starts threads to answer for the actors, and threads count
# against the limit as processes do, so it must leave room to start a worker again
# when one is gone. It is run twice: where the server sees its own control group as the
# top one, as a container mostly does, and where it sees the groups of its whole
# machine and its own is further in (`--cgroupns=host`).
#
#     sh tests/stress/limits.sh
#
# It needs Docker, and an image with a Rust toolchain, clang, curl and procps. What it
# is given by the environment: DOCKER (the command, `docker`), IMAGE (`koo-dev`),
# TARGET (a volume for what is built, `koo-target`), LIMIT (the processes allowed, 40).
DOCKER=${DOCKER:-docker}
IMAGE=${IMAGE:-koo-dev}
TARGET=${TARGET:-koo-target}
LIMIT=${LIMIT:-40}
root=$(cd "$(dirname "$0")/../.." && pwd)
failed=0

$DOCKER run --rm -v "$root":/src:ro -v "$TARGET":/target "$IMAGE" sh -c '
    mkdir -p /koo && cd /src && tar --exclude=./target --exclude=./.git --exclude=./.claude -cf - . | tar -xf - -C /koo &&
    cd /koo && cargo build --release -q && /target/release/koo build --release tests/stress/check.koo -o /target/check-limits
' || { echo "FAIL: the server was not built"; exit 1; }

# What is asked of the server's container is asked from another that shares its
# processes and its network, and not its limit: a command run inside it would count
# against the limit, and where the server has left room for one worker and no more,
# would not start.
beside() {
    $DOCKER run --rm --pid container:$name --network container:$name "$IMAGE" sh -c "$1"
}

for groups in private host; do
    name=koo-limits-$groups
    $DOCKER rm -f $name >/dev/null 2>&1
    $DOCKER run -d --name $name --cgroupns=$groups --pids-limit "$LIMIT" -e PORT=39800 -e WORKERS=4 \
        -v "$TARGET":/target:ro "$IMAGE" /target/check-limits >/dev/null || { echo "FAIL ($groups): the server did not start"; failed=1; continue; }
    sleep 2
    # As many asks at once, of as many actors, as would have the watching process start
    # every thread it may: from another container, whose processes are not counted here.
    $DOCKER run -d --rm --name $name-load --network container:$name "$IMAGE" sh -c '
        for round in 1 2 3; do
            seq 1 200 | xargs -P 200 -I{} curl -s -m 20 -o /dev/null localhost:39800/nap/{}/2500
        done' >/dev/null
    sleep 3
    before=$(beside 'pgrep -P 1 | wc -l')
    threads=$(beside 'ls /proc/1/task | wc -l')
    beside 'kill -9 $(pgrep -P 1 | head -1)'
    sleep 4
    after=$(beside 'pgrep -P 1 | wc -l')
    answered=$(beside 'curl -s -m 5 localhost:39800/ping')
    echo "$groups: limit $LIMIT, workers $before, threads of the watching process $threads, workers after one was killed $after, ping: $answered"
    if [ "$before" != 4 ] || [ "$after" != 4 ] || [ "$answered" != pong ]; then
        echo "FAIL ($groups): a worker that was killed under a limit of $LIMIT processes was not started again"
        $DOCKER logs $name 2>&1 | tail -5
        failed=1
    fi
    # A server that does not see its limit starts threads until the system refuses
    # one, and then has a worker again only if it is quicker to start one than its
    # own threads are to take the room: it must have left the room.
    if [ $((threads + before)) -ge "$LIMIT" ]; then
        echo "FAIL ($groups): the threads and workers are $((threads + before)), the whole of the limit of $LIMIT: no room was left to start a worker in"
        failed=1
    fi
    $DOCKER rm -f $name-load $name >/dev/null 2>&1
done
[ $failed = 0 ] && echo "all passed"
exit $failed
