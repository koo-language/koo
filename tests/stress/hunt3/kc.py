# A process that is not a worker sends the supervising process one well-formed message that names an
# actor by the number of an actor of another kind.
import sys, time, subprocess
sys.path.insert(0, '/x')
import raw
which = sys.argv[1] if len(sys.argv) > 1 else 'all'
def http(path):
    return subprocess.run(['curl', '-s', '-m', '3', '-w', ' [%{http_code}]', 'http://127.0.0.1:8080' + path], capture_output=True, text=True).stdout.strip()
print('before: /next ->', http('/next'), ' /names ->', http('/names'))
s = raw.conn()
method = {'all': 'Names 0 all', 'add': 'Names 0 add', 'next': 'Counter 1 next'}[which]
body = '["bob"]' if which == 'add' else '[]'
print('sent to the socket %s: %r %s' % (raw.name(), method, body))
s.sendall(raw.msg(0, method, body)); s.settimeout(3)
try: print('   answer:', s.recv(200))
except Exception as e: print('   answer:', repr(e))
time.sleep(0.5)
print('after:  /next ->', http('/next'), ' /names ->', http('/names'))
