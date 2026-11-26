cp /etc/hosts /tmp/hosts.orig
python3 /x/web.py > /dev/null 2>&1 &
python3 /x/hd.py 127.0.0.2 ok > /tmp/dns-ok.log 2>&1 &
python3 /x/hd.py 127.0.0.3 silent > /tmp/dns-silent.log 2>&1 &
python3 /x/hd.py ::1 ok > /tmp/dns-v6.log 2>&1 &
python3 /x/hd.py 127.0.0.4 byname > /tmp/dns-byname.log 2>&1 &
python3 /x/hd.py 127.0.0.5 ednsdrop > /tmp/dns-ednsdrop.log 2>&1 &
sleep 0.8
t() { # t "label" "resolv.conf with ; for newlines" "extra hosts lines with ;" host ms
  printf '%b' "$(echo "$2" | sed 's/;/\\n/g')" > /etc/resolv.conf
  cp /tmp/hosts.orig /etc/hosts; printf '%b' "$(echo "$3" | sed 's/;/\\n/g')" >> /etc/hosts
  : > /tmp/dns-ok.log; : > /tmp/dns-v6.log; : > /tmp/dns-byname.log
  echo "$1: $(/x/look $4 $5 2>&1 | tail -1)   [asked: $(grep -ho 'asked [^ ]* [A]*' /tmp/dns-ok.log /tmp/dns-v6.log /tmp/dns-byname.log | tr '\n' ',' | cut -c1-150)]"
}
t "plain" "nameserver 127.0.0.2;" "" a.test 3000
t "no newline at the end" "nameserver 127.0.0.2" "" a.test 3000
t "CRLF line ends" "nameserver 127.0.0.2\r;" "" a.test 3000
t "tabs and trailing blanks" "nameserver\t127.0.0.2  \t;" "" a.test 3000
t "comment with ;" "; nameserver 127.0.0.3;nameserver 127.0.0.2 ; the one;" "" a.test 3000
t "comment with # glued" "nameserver 127.0.0.2#x;" "" a.test 3000
t "four servers, the fourth is the good one" "nameserver 127.0.0.3;nameserver 127.0.0.3;nameserver 127.0.0.3;nameserver 127.0.0.2;" "" a.test 3000
t "silent then good" "nameserver 127.0.0.3;nameserver 127.0.0.2;" "" a.test 3000
t "address with a port (not valid)" "nameserver 127.0.0.2:53;" "" a.test 1500
t "empty file" "" "" a.test 1500
t "v6 loopback server" "nameserver ::1;" "" a.test 3000
t "v6 and v4 servers" "nameserver ::1;nameserver 127.0.0.3;" "" a.test 3000
t "v4-mapped server" "nameserver ::ffff:127.0.0.2;" "" a.test 3000
t "v4-mapped and v4 silent" "nameserver ::ffff:127.0.0.2;nameserver 127.0.0.3;" "" a.test 3000
t "v6 with scope" "nameserver fe80::1%eth0;nameserver 127.0.0.2;" "" a.test 3000
t "v6 unreachable and v4 good" "nameserver 2001:db8::1;nameserver 127.0.0.2;" "" a.test 3000
t "v6 unreachable only" "nameserver 2001:db8::1;" "" a.test 3000
t "search, one label" "nameserver 127.0.0.4;search test example;" "" ok 3000
t "search, name with a dot ending" "nameserver 127.0.0.4;search test;" "" ok.test. 3000
t "ndots:5 as in a cluster" "nameserver 127.0.0.4;search a.test b.test c.test;options ndots:5;" "" ok.test 3000
t "ndots of thirty digits" "nameserver 127.0.0.4;search test;options ndots:999999999999999999999999999999;" "" ok.test 3000
t "ndots:0" "nameserver 127.0.0.4;search test;options ndots:0;" "" ok 3000
t "forty search domains, all silent" "nameserver 127.0.0.4;search $(for i in $(seq 1 40); do printf 'silent%d.x ' $i; done);" "" silent 3000
t "domain and search both" "nameserver 127.0.0.4;domain nx.x;search test;" "" ok 3000
t "search of a dot" "nameserver 127.0.0.4;search .;" "" ok.test 3000
t "options rotate timeout:1 attempts:1" "nameserver 127.0.0.3;nameserver 127.0.0.2;options rotate timeout:1 attempts:1;" "" a.test 3000
t "old server that drops long questions" "nameserver 127.0.0.5;" "" a.test 5000
t "hosts: plain" "nameserver 127.0.0.3;" "127.0.0.1 a.test;" a.test 1500
t "hosts: other case, tab, alias" "nameserver 127.0.0.3;" "127.0.0.1\tfirst.test A.TEST;" a.test 1500
t "hosts: v6 only" "nameserver 127.0.0.3;" "::1 a.test;" a.test 1500
t "hosts: v6 then v4" "nameserver 127.0.0.3;" "::1 a.test;127.0.0.1 a.test;" a.test 1500
t "hosts: comment" "nameserver 127.0.0.2;" "#127.0.0.9 a.test;127.0.0.1 b.test # a.test;" a.test 1500
t "hosts: CRLF" "nameserver 127.0.0.3;" "127.0.0.1 a.test\r;" a.test 1500
t "hosts: no newline at end" "nameserver 127.0.0.3;" "127.0.0.1 a.test" a.test 1500
t "hosts: name with a dot ending asked" "nameserver 127.0.0.3;" "127.0.0.1 a.test;" a.test. 1500
t "hosts: bad address" "nameserver 127.0.0.2;" "999.0.0.1 a.test;" a.test 1500
t "hosts: 0.0.0.0" "nameserver 127.0.0.3;" "0.0.0.0 a.test;" a.test 1500
# a hosts file of more than 64 KB: the entry is after the cut, and one straddles it
python3 - <<'PY'
lines = ''.join('10.0.0.%d filler%d.test\n' % (i % 250, i) for i in range(4000))
cut = 65536
pre = lines[:cut - 30]
pre = pre[:pre.rfind('\n') + 1]
entry = '127.0.0.1 straddle.test\n'
pad = cut - len(pre) - len('127.0.0.1 strad')
pre += '#' * (pad - 1) + '\n'
open('/tmp/bighosts', 'w').write(pre + entry + '127.0.0.1 after.test\n')
PY
cp /tmp/bighosts /etc/hosts; echo "nameserver 127.0.0.3" > /etc/resolv.conf
echo "hosts over 64 KB, entry after the cut: $(/x/look after.test 1500)"
echo "hosts over 64 KB, a name cut in two (strad|dle.test), asked 'strad': $(/x/look strad 1500)"
echo "hosts over 64 KB, the whole name asked: $(/x/look straddle.test 1500)"
