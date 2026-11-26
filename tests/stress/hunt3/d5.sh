echo "nameserver 127.0.0.2" > /etc/resolv.conf
sleep 1
for i in 1 2 3; do /x/look silent.test 1000; done
for i in 1 2 3 4 5; do /x/look flood8.test 1000; sleep 9; done
