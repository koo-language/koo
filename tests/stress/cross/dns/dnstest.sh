#!/bin/sh
apk add --no-cache dnsmasq >/dev/null 2>&1 || { echo "apk failed"; exit 1; }
cat /x/dns/$CONF.conf > /etc/resolv.conf; dnsmasq -C /x/dns/dnsmasq.conf || exit 1
/x/n/srv-a >/dev/null &
sleep 0.7
for h in "$@"; do
  echo "-- $h: $(/x/n/get-a http://$h:8080/ 3 2>&1 | tr '\n' ' ' | cut -c1-150)"
done
echo "--- queries seen by dnsmasq:"; grep -c "query\[" /tmp/dnsmasq.log; grep "query\[" /tmp/dnsmasq.log | awk '{print $5, $6}' | tr '\n' ';' | cut -c1-1500
