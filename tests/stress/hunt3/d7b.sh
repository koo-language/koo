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
  cat /tmp/hosts.orig > /etc/hosts; printf '%b' "$(echo "$3" | sed 's/;/\\n/g')" >> /etc/hosts
  : > /tmp/dns-ok.log; : > /tmp/dns-v6.log; : > /tmp/dns-byname.log
  echo "$1: $(/x/look $4 $5 2>&1 | tail -1)   [asked: $(grep -ho 'asked [^ ]* [A]*' /tmp/dns-ok.log /tmp/dns-v6.log /tmp/dns-byname.log | tr '\n' ',' | cut -c1-150)]"
}
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
cat /tmp/bighosts > /etc/hosts; echo "nameserver 127.0.0.3" > /etc/resolv.conf
echo "hosts over 64 KB, entry after the cut: $(/x/look after.test 1500)"
echo "hosts over 64 KB, a name cut in two (strad|dle.test), asked 'strad': $(/x/look strad 1500)"
echo "hosts over 64 KB, the whole name asked: $(/x/look straddle.test 1500)"
