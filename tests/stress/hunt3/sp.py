# A process that is not a worker, on the supervising process's socket:
# (1) one byte too many while an actor is busy; (2) a long answer asked for and never read.
import sys, time, subprocess, os
sys.path.insert(0, '/x')
import raw
SUP = int(sys.argv[1])
def ticks():
    f = open('/proc/%d/stat' % SUP).read().split(); return int(f[13]) + int(f[14])
def cpu(sec):
    a = ticks(); time.sleep(sec); return ticks() - a
def http(path, m=3):
    return subprocess.run(['curl', '-s', '-m', str(m), '-w', '[%{http_code} %{time_total}s]', 'http://127.0.0.1:8080' + path], capture_output=True, text=True).stdout.replace('\n', ' ')
print('idle: the supervising process used %d hundredths of a second of processor in 3 s' % cpu(3))
s = raw.conn(); s.sendall(raw.msg(0, 'Counter 5 nap', '[8000]')); time.sleep(0.3)
print('an actor is asked to sleep 8 s:                 %d hundredths in 3 s' % cpu(3))
s.sendall(b'\x00')
print('one more byte is sent on the same connection:   %d hundredths in 3 s' % cpu(3))
time.sleep(2.5)
print('after the actor has answered:                   %d hundredths in 3 s' % cpu(3))
s.close()
s = raw.conn(); s.sendall(raw.msg(0, 'Counter 8 big', '[3000000]')); time.sleep(0.5)
print('a 3 MB answer is asked for and not read; then /next/8 ->', http('/next/8'), ' /next/9 ->', http('/next/9'))
time.sleep(5)
print('5 s later: /next/8 ->', http('/next/8'))
s.close(); time.sleep(0.5)
print('after that connection is closed: /next/8 ->', http('/next/8'))
