python3 /x/web.py > /dev/null 2>&1 &
python3 /x/hd.py 127.0.0.2 ok > /tmp/dns-ok.log 2>&1 &
python3 /x/hd.py 127.0.0.3 silent > /tmp/dns-silent.log 2>&1 &
python3 /x/hd.py 127.0.0.4 byname > /tmp/dns-byname.log 2>&1 &
sleep 0.8
echo "v6 addresses here: $(ip -6 addr 2>/dev/null | grep -c inet6)"
t() { printf '%b' "$(echo "$2" | sed 's/;/\\n/g')" > /etc/resolv.conf; echo "$1: $(/x/look $3 $4 2>&1 | tail -1)"; }
t "v6 loopback and v4 good" "nameserver ::1;nameserver 127.0.0.2;" a.test 3000
t "v6 global and v4 good" "nameserver 2001:db8::1;nameserver 127.0.0.2;" a.test 3000
t "v6 first, v4 silent, v4 good" "nameserver 2001:db8::1;nameserver 127.0.0.3;nameserver 127.0.0.2;" a.test 3000
t "v6 only" "nameserver ::1;" a.test 3000
t "v4-mapped good" "nameserver ::ffff:127.0.0.2;" a.test 3000
t "v6 and v4, answer cut short (over a connection)" "nameserver 2001:db8::1;nameserver 127.0.0.4;" tcnx.test 3000
t "v4-mapped, answer cut short (over a connection)" "nameserver ::ffff:127.0.0.4;" tcnx.test 3000
t "host has only a v6 address" "nameserver 127.0.0.4;" aaaaonly.test 3000
