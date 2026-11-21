# Spawn Keywhere (dev build) as our child, scan its memory for known secrets
# while unlocked, then after `lock`. Run inside a nested KWin session.
import os, re, subprocess, sys, time, socket
M = sys.argv[1]; APP = sys.argv[2]; mode = sys.argv[3]  # mode: hardened | scan
secrets = [s for s in open(M + '/secrets.txt').read().split('\n') if s]
env = dict(os.environ, KEYWHERE_DEMO_PASSWORD='test123', XDG_CONFIG_HOME=M + '/cfg', HOME=M)
if mode == 'scan':
    env['KEYWHERE_DEV_MEMORY_TEST'] = '1'
args=[APP, M + '/demo.psafe3'] + ([M + '/pw.txt'] if mode == 'ui' else []) + (['copy:Gmail'] if mode == 'copy' else []) + (['reveal:Company SSO'] if mode == 'reveal' else [])
if mode in ('copy', 'reveal'):
    env['KEYWHERE_DEV_MEMORY_TEST'] = '1'
if mode == 'ui':
    env['KEYWHERE_DEV_MEMORY_TEST'] = '1'
    env.pop('KEYWHERE_DEMO_PASSWORD', None)
p = subprocess.Popen(args, env=env, stdout=subprocess.DEVNULL, stderr=open(M + '/stderr-' + mode + '.txt', 'w'))
out = open(M + '/result-' + mode + '.txt', 'w')
def log(*a):
    print(*a, file=out, flush=True)
time.sleep(5)

def scan(label):
    try:
        maps = open(f'/proc/{p.pid}/maps').read().splitlines()
        mem = open(f'/proc/{p.pid}/mem', 'rb', 0)
    except Exception as e:
        log(label, 'CANNOT READ MEMORY:', type(e).__name__, e)
        return
    found = {}
    total = 0
    for line in maps:
        parts = line.split()
        addr, perms = parts[0], parts[1]
        if 'r' not in perms or (len(parts) > 5 and parts[5] in ('[vvar]', '[vsyscall]')):
            continue
        a, b = (int(x, 16) for x in addr.split('-'))
        try:
            mem.seek(a); data = mem.read(b - a)
        except Exception:
            continue
        total += len(data)
        for s in secrets:
            for enc in (s.encode('utf-8'), s.encode('utf-16-le')):
                c = data.count(enc)
                if c:
                    where = parts[5] if len(parts) > 5 else '[anon]'
                    found.setdefault(s, []).append((c, where, 'utf16' if enc != s.encode() else 'utf8'))
                    if label.startswith('AFTER') and s != 'test123' and '+4' in label:
                        i = data.find(enc)
                        while i >= 0:
                            ctx = data[max(0,i-160):i+len(enc)+96]
                            log('  CTX @%x' % (a+i), ctx.hex())
                            log('  TXT', ''.join(chr(x) if 32<=x<127 else '.' for x in ctx))
                            i = data.find(enc, i+1)
    log(f'{label}: scanned {total/1e6:.1f} MB; secrets present: {len(found)}/{len(secrets)}')
    for s, hits in sorted(found.items()):
        log('   ', ('*' * 3 + s[-3:]) if s != 'test123' else 'MASTER(test123)', hits)

scan('UNLOCKED')
sock = '/tmp/keywhere-%d/keywhere-dev.sock' % os.getuid()
try:
    c = socket.socket(socket.AF_UNIX); c.connect(sock); c.sendall(b'lock\n'); c.close()
    log('sent lock')
except Exception as e:
    log('lock send failed', e)
time.sleep(4)
scan('AFTER LOCK (+4s)')
time.sleep(6)
scan('AFTER LOCK (+10s)')
p.terminate(); p.wait()
