#!/bin/sh
# run inside the container: sets up name servers and a web server, then runs what is asked
# usage: run1.sh "<resolv.conf lines separated by ;>" "<servers: addr=who,addr=who>" look-args...
conf="$1"; servers="$2"; shift 2
echo "$conf" | tr ';' '\n' > /etc/resolv.conf
python3 /x/web.py &
for s in $(echo "$servers" | tr ',' ' '); do
  python3 /x/dns.py "${s%%=*}" "${s##*=}" &
done
sleep 0.7
"$@"
