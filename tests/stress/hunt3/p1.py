import sys, os, time, struct, socket, subprocess
sys.path.insert(0, '/x')
import raw
SUP = int(sys.argv[1])
def ticks():
    f = open('/proc/%d/stat' % SUP).read().split()
    return int(f[13]) + int(f[14])
def cpu(sec=2.0):
    a = ticks(); time.sleep(sec); return ticks() - a
def alive():
    try:
        out = subprocess.run(['curl', '-s', '-m', '5', 'http://127.0.0.1:8080/next/63'], capture_output=True).stdout
        return out.strip().decode()
    except Exception as e:
        return 'ERR %r' % e
def st():
    try:
        return 'fds=%d threads=%d rss=%s' % (len(os.listdir('/proc/%d/fd' % SUP)), len(os.listdir('/proc/%d/task' % SUP)),
            [l.split()[1] for l in open('/proc/%d/status' % SUP) if l.startswith('VmRSS')][0])
    except Exception as e:
        return 'SUP GONE %r' % e
def t(label, fn):
    try:
        r = fn()
    except Exception as e:
        r = 'EXC %r' % e
    print('%-34s -> %-60s | next=%s cpu2s=%d %s' % (label, str(r)[:60], alive(), cpu(), st()), flush=True)

def send_and_wait(data, wait=1.0, readit=True):
    s = raw.conn(); s.sendall(data); s.settimeout(wait)
    try:
        d = s.recv(4096) if readit else b''
        return ('got', d[:40])
    except socket.timeout:
        return 'no reply (timeout)'
    finally:
        keep.append(s)
keep = []
print('baseline', alive(), cpu(), st())
t('valid next', lambda: raw.ask(raw.conn(), 'Counter 0 next'))
t('random 64 bytes', lambda: send_and_wait(os.urandom(64)))
t('size 0', lambda: send_and_wait(struct.pack('<I', 0)))
t('size 1', lambda: send_and_wait(struct.pack('<I', 1) + b'\0'))
t('size 2', lambda: send_and_wait(struct.pack('<I', 2) + b'\0\0'))
t('size 3 empty method', lambda: send_and_wait(struct.pack('<I', 3) + b'\0\0\0'))
t('size 0xffffffff then stall', lambda: send_and_wait(struct.pack('<I', 0xffffffff) + b'\0\5\0'))
t('size 0x7fffffff then stall', lambda: send_and_wait(struct.pack('<I', 0x7fffffff) + b'\0\5\0'))
t('method len beyond size', lambda: send_and_wait(struct.pack('<I', 5) + b'\0\xff\xffab'))
t('kind 7', lambda: send_and_wait(raw.msg(7, 'Counter 0 next', '[]')))
t('kind 1 (send) of ask method', lambda: send_and_wait(raw.msg(1, 'Counter 0 next', '[]')))
t('kind 2 (answered)', lambda: send_and_wait(raw.msg(2, 'Counter 0 next', '[]')))
t('no such method', lambda: send_and_wait(raw.msg(0, 'Counter 0 nomethod', '[]')))
t('no such actor id', lambda: send_and_wait(raw.msg(0, 'Counter 99999 next', '[]')))
t('negative id', lambda: send_and_wait(raw.msg(0, 'Counter -5 next', '[]')))
t('huge id', lambda: send_and_wait(raw.msg(0, 'Counter 9223372036854775807 next', '[]')))
t('non-numeric id', lambda: send_and_wait(raw.msg(0, 'Counter x next', '[]')))
t('unknown kind', lambda: send_and_wait(raw.msg(0, 'Nope 0 next', '[]')))
t('two parts', lambda: send_and_wait(raw.msg(0, 'Counter 0', '[]')))
t('four parts', lambda: send_and_wait(raw.msg(0, 'Counter 0 next x', '[]')))
t('bad json', lambda: send_and_wait(raw.msg(0, 'Counter 0 echo', '[notjson')))
t('wrong type', lambda: send_and_wait(raw.msg(0, 'Counter 0 echo', '[5]')))
t('missing arg', lambda: send_and_wait(raw.msg(0, 'Counter 0 echo', '[]')))
t('extra arg', lambda: send_and_wait(raw.msg(0, 'Counter 0 next', '[1,2,3]')))
t('empty body', lambda: send_and_wait(raw.msg(0, 'Counter 0 next', '')))
t('deep json 200k', lambda: send_and_wait(raw.msg(0, 'Counter 0 echo', '[' * 200000)))
t('big n', lambda: send_and_wait(raw.msg(0, 'Counter 0 big', '[-1]')))
t('nul in method', lambda: send_and_wait(raw.msg(0, b'Counter 0 ne\0xt', '[]')))
t('.start with args', lambda: send_and_wait(raw.msg(0, 'Counter 0 .start', '[]')))
t('.start Registry', lambda: send_and_wait(raw.msg(0, 'Registry 0 .start', '[]')))
t('stop then next', lambda: (send_and_wait(raw.msg(0, 'Counter 62 stop', '[]')), send_and_wait(raw.msg(0, 'Counter 62 next', '[]'))))
t('try.tell', lambda: send_and_wait(raw.msg(0, 'Counter 0 try.tell', '[]')))
t('try.next (not tellable)', lambda: send_and_wait(raw.msg(0, 'Counter 0 try.next', '[]')))
# a slow ask, then one more byte while it is busy
def early():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 5 nap', '[4000]')); time.sleep(0.3); s.sendall(b'\x01'); keep.append(s)
    c = cpu(2.0)
    return 'cpu while busy with early byte: %d ticks/2s' % c
t('early byte on busy conn', early)
time.sleep(3)
def early_whole():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 5 nap', '[4000]')); time.sleep(0.3); s.sendall(raw.msg(0, 'Counter 5 next', '[]')); keep.append(s)
    c = cpu(2.0)
    return 'cpu while busy with early message: %d ticks/2s' % c
t('early message on busy conn', early_whole)
time.sleep(3)
def held_early():
    s1 = raw.conn(); s1.sendall(raw.msg(0, 'Counter 6 nap', '[4000]')); time.sleep(0.2)
    s2 = raw.conn(); s2.sendall(raw.msg(0, 'Counter 6 next', '[]')); time.sleep(0.2); s2.sendall(b'\x01'); keep.extend([s1, s2])
    c = cpu(2.0)
    return 'cpu with early byte on held-up conn: %d ticks/2s' % c
t('early byte on held-up conn', held_early)
time.sleep(3)
def half():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 0 echo', '["' + 'x' * 100000 + '"]')[:5000]); keep.append(s)
    return 'half message parked'
t('half message', half)
def half_then_close():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 0 echo', '["' + 'x' * 100000 + '"]')[:5000]); s.close()
    return 'half message closed'
t('half message then close', half_then_close)
def ask_then_close():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 7 nap', '[1000]')); s.close(); time.sleep(1.5)
    return 'closed while busy'
t('close while busy', ask_then_close)
def big_then_close():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 7 big', '[3000000]')); s.close(); time.sleep(0.5)
    return 'closed before big answer'
t('close before big answer', big_then_close)
def big_never_read():
    s = raw.conn(); s.sendall(raw.msg(0, 'Counter 8 big', '[3000000]')); keep.append(s); time.sleep(0.5)
    r = subprocess.run(['curl', '-s', '-m', '5', '-w', '[%{http_code} %{time_total}]', 'http://127.0.0.1:8080/next/8'], capture_output=True).stdout
    r2 = subprocess.run(['curl', '-s', '-m', '5', '-w', '[%{http_code} %{time_total}]', 'http://127.0.0.1:8080/next/9'], capture_output=True).stdout
    return 'same actor: %s other: %s' % (r, r2)
t('big answer never read', big_never_read)
print('final', alive(), cpu(), st())
for s in keep: s.close()
time.sleep(1)
print('after closing all', alive(), cpu(), st())
