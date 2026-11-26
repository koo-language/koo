#!/bin/sh
# inside the container: name server, web server, sinks, fake database, the Koo server; then the driver
echo "nameserver 127.0.0.2" > /etc/resolv.conf
python3 /x/web.py &
python3 /x/dns.py 127.0.0.2 byname > /dev/null &
python3 /x/sinks.py 8080 &
python3 /x/fakepg_c.py 39600 > /dev/null &
/x/hsrv &
sleep 1
python3 /x/drive.py "$@"
