"""An original foreground proves retained RAM, cwd and open descriptions."""
import json
import re
import shlex
import time
from uuid import uuid4

from helpers.service import exec_output_text

from .route_lifecycle import validate_exec
from .stream_measurements import service_gateway
from .stream_protocol import open_stream, read_until, ready_terminal, stdin

FILE = 'capsem-route-warm-file'
INFO = 'capsem-route-warm-info'
RUNTIME = '/run/capsem-route-warm-runtime'
ORIGINAL = b'original-warm-file'
RESUMED = ORIGINAL+b'-resumed'


def foreground(nonce):
    """Same physical descriptor probes as terminal_suspend; no host execution."""
    assert re.fullmatch('[0-9a-f]{32}',nonce)
    return f'''
import json,os,termios,tty
from pathlib import Path
assert int(Path('/proc/self/uid_map').read_text().split()[1]) > 0
os.chdir('/workspace')
Path({FILE!r}).write_bytes({ORIGINAL!r})
directory=os.open('.',os.O_RDONLY|os.O_DIRECTORY)
held=os.open({FILE!r},os.O_RDWR)
image=os.open('/etc/os-release',os.O_RDONLY)
identity=lambda fd:(os.fstat(fd).st_dev,os.fstat(fd).st_ino)
before_file,before_directory=identity(held),identity(directory)
image_bytes=os.read(image,4096)
memory=os.urandom(16).hex()
Path({INFO!r}).write_text(json.dumps(dict(pid=os.getpid(),memory=memory,nonce={nonce!r})))
before=termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1,b'\\x1b[2J\\x1b[Hwarm-ready-'+{nonce!r}.encode()+b'\\n')
    assert os.read(0,1)==b'c'
    assert os.getcwd()=='/workspace'
    assert identity(held)==before_file and identity(directory)==before_directory
    assert os.read(held,4096)=={ORIGINAL!r}
    assert os.lseek(image,0,os.SEEK_SET)==0 and os.read(image,4096)==image_bytes
    assert Path('/etc/os-release').read_bytes()==image_bytes
    os.write(held,b'-resumed'); os.fsync(held)
    relative=os.open({FILE!r},os.O_RDONLY,dir_fd=directory)
    try:
        assert os.read(relative,4096)=={RESUMED!r}
    finally:
        os.close(relative)
    assert Path({FILE!r}).read_bytes()=={RESUMED!r}
    os.write(1,b'\\x1b[2J\\x1b[Hwarm-restored-'+memory.encode()+b'\\n')
    assert os.read(0,1)==b'q'
    os.write(1,b'\\x1b[2J\\x1b[Hwarm-finished-'+memory.encode()+b'\\n')
finally:
    termios.tcsetattr(0,termios.TCSANOW,before)
    for fd in (held,directory,image): os.close(fd)
'''


class WarmProbe:
    def __init__(self,cycle,vm_id):
        self.cycle,self.vm_id,self.nonce = cycle,vm_id,uuid4().hex
        self.gateway = service_gateway(cycle.service)
        self.state = None

    def connection(self,deadline):
        return open_stream(self.gateway,self.vm_id,{'type':'start','kind':'terminal'},
            deadline=deadline,close_timeout=self.cycle.cfg['close_timeout_seconds'])

    def limits(self):
        return {'max_frames':self.cycle.cfg['max_probe_frames'],'max_bytes':self.cycle.cfg['max_probe_bytes']}

    def expect(self,stream,marker,deadline):
        remaining = deadline-time.monotonic()
        assert remaining > 0, 'original warm response deadline exceeded'
        read_until(stream,marker,remaining,**self.limits())

    def metadata(self,deadline,*,alive=False):
        command = f"import json,os; from pathlib import Path; p=Path('/workspace/{INFO}'); data=p.read_bytes(); assert len(data)<={self.cycle.cfg['max_metadata_bytes']}; value=json.loads(data); "
        if alive:
            command += "os.kill(value['pid'],0); "
        command += 'print(json.dumps(value,sort_keys=True))'
        _,result = self.cycle.execute(self.vm_id,'python3 -c '+shlex.quote(command),deadline=deadline)
        _,separator,text = exec_output_text(result).partition('\n')
        assert separator and len(text.encode()) <= self.cycle.cfg['max_metadata_bytes']
        validate_exec(result,text.rstrip('\n'))
        value = json.loads(text)
        assert set(value) == {'pid','memory','nonce'} and type(value['pid']) is int and value['pid'] > 0
        assert value['nonce'] == self.nonce and re.fullmatch('[0-9a-f]{32}',value['memory'])
        return value

    def start(self,deadline):
        with self.connection(deadline) as stream:
            ready_terminal(stream,deadline=deadline,**self.limits())
            stream.send(stdin(('python3 -c '+shlex.quote(foreground(self.nonce))+'\n').encode()))
            self.expect(stream,('warm-ready-'+self.nonce).encode(),deadline)
            self.state = self.metadata(deadline,alive=True)

    def resumed(self,deadline):
        assert self.state is not None, 'original warm probe was never started'
        with self.connection(deadline) as stream:
            # This must be the ORIGINAL foreground, not a newly spawned exec.
            stream.send(stdin(b'c'))
            self.expect(stream,('warm-restored-'+self.state['memory']).encode(),deadline)
            assert self.metadata(deadline,alive=True) == self.state, 'foreground identity changed'
            stream.send(stdin(b'q'))
            self.expect(stream,('warm-finished-'+self.state['memory']).encode(),deadline)
