echo "nameserver 127.0.0.2" > /etc/resolv.conf
python3 /x/web.py > /dev/null 2>&1 &
QUIET=1 python3 /x/hd.py 127.0.0.2 byname 2>&1 | grep --line-buffered '^.flood' &
sleep 3600
