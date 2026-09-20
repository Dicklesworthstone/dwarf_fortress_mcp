#!/usr/bin/env python3
"""Reviewed one-shot workforce control and durable receipt recovery (development only).

No MCP server, game job completion, production admission, automatic retries,
evidence repair, or unrequested labor configuration is provided.
"""
from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import time

import workforce_wire as w

FORMAT = 'dfmcp.workforce-journal/1'
MAX_BYTES, MAX_LINE, MAX_EVENTS, MAX_KEYS = 64*1024*1024, 160*1024, 512, 64
OUTPUT_LIMIT = 128*1024
STATES = ('intent','prepared','dispatch_started','tracking','cancel_requested','terminal','cancelled_before_dispatch')
ALLOWED = {'DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17','DFMCP_WORKFORCE_TOKEN',
           'DFMCP_WORKFORCE_ENDPOINT','DFMCP_WORKFORCE_ALLOW_LABOR'}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode('ascii')


def unique_pairs(pairs):
    result = {}
    for k,v in pairs:
        w.require(k not in result, 'duplicate JSON key'); result[k] = v
    return result


def wrapped(value):
    digest = hashlib.sha256(canonical(value)).hexdigest()
    return canonical({'value':value,'sha256':digest}) + b'\n', digest


def decoded(line):
    w.require(1 <= len(line) <= MAX_LINE and line.endswith(b'\n'), 'incomplete or oversized journal line')
    envelope = json.loads(line, object_pairs_hook=unique_pairs)
    w.require(type(envelope) is dict and set(envelope) == {'value','sha256'}, 'invalid journal envelope')
    exact, digest = wrapped(envelope['value'])
    w.require(line == exact and envelope['sha256'] == digest, 'noncanonical or corrupt journal line')
    return envelope['value'], digest


def validate_binding(b):
    w.require(type(b) is dict and set(b) == {'endpoint','generation','folder','site','df_version','dfhack_version'}, 'invalid journal binding')
    w.address(b['endpoint']); w.integer(b['generation'],1,2**64-2); w.text(b['folder'],512); w.integer(b['site'],0,2**31-1)
    w.text(b['df_version'],128); w.text(b['dfhack_version'],128)
    return b


def settled(entry):
    return entry['state'] in ('terminal','cancelled_before_dispatch')


def transition(old, entry, binding):
    w.require(type(entry) is dict and set(entry) == {'plan','state','effect_hex'}, 'invalid assignment journal entry')
    plan = w.validate_plan(entry['plan']); state = entry['state']
    w.require(state in STATES, 'unknown coordinator state')
    before = w.capture(bytes.fromhex(plan['capture_hex']))
    w.require(all(before[k] == binding[k] for k in ('generation','folder','site')), 'entry from another fortress')
    record = None if entry['effect_hex'] is None else w.effect(w.unhex(entry['effect_hex'],1,w.MAX_EFFECT),plan)
    if state in ('intent','cancelled_before_dispatch'):
        w.require(record is None, 'local intent/cancel has native receipt')
    else:
        w.require(record is not None, 'coordinator state requires retained native evidence')
        if state in ('prepared','dispatch_started'):
            w.require(record['phase'] == 'prepared', 'prepared coordination without native preparation')
        elif state in ('tracking','cancel_requested'):
            w.require(record['phase'] in ('prepared','unknown'), 'tracking cannot contain terminal receipt')
        else:
            w.require(record['phase'] in ('applied','refused','cancelled'), 'terminal receipt missing')
    if old is None:
        w.require(state == 'intent', 'first event must record intent')
        return
    w.require(old['plan'] == plan, 'assignment key was rebound')
    permitted = {'intent':{'prepared','tracking','terminal','cancelled_before_dispatch'},
                 'prepared':{'dispatch_started','cancelled_before_dispatch'},
                 'dispatch_started':{'tracking','terminal','cancel_requested'},
                 'tracking':{'tracking','terminal','cancel_requested'}, 'cancel_requested':{'tracking','terminal'}}
    w.require(state in permitted.get(old['state'],set()), 'illegal assignment transition or redispatch')
    previous = None if old['effect_hex'] is None else w.effect(bytes.fromhex(old['effect_hex']),plan)
    if previous and previous['phase'] == 'unknown':
        # Native Unknown is permanent for this engine; no later read infers success.
        w.require(record is not None and entry['effect_hex'] == old['effect_hex'], 'native Unknown receipt changed')
    if state in ('dispatch_started','cancel_requested'):
        w.require(entry['effect_hex'] == old['effect_hex'], 'dispatch marker changed preparation')


def check_time(deadline):
    w.require(time.monotonic() < deadline, 'shared operation deadline exhausted')


@contextmanager
def parent_fd(path):
    w.require(os.name == 'posix' and hasattr(os,'O_NOFOLLOW'), 'private journal custody requires POSIX')
    w.require(path.is_absolute() and path.name and '..' not in path.parts and len(str(path)) <= 4096,
              'absolute bounded journal path without traversal required')
    fd = os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=fd)
            os.close(fd); fd = child
        info = os.fstat(fd)
        w.require(stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid in (0,os.geteuid()), 'journal directory must be private mode 0700')
        yield fd
    finally:
        os.close(fd)


class Journal:
    """Bounded, append-only, exclusively owned coordination, not game history."""
    def __init__(self,path,deadline,writable=False,create_binding=None):
        self.path, self.deadline, self.writable = Path(path), deadline, writable
        self.create_binding, self.fd, self.parent, self.fenced = create_binding, None, None, False
        self.raw, self.entries, self.events = b'', {}, 0

    def __enter__(self):
        import fcntl
        check_time(self.deadline)
        self.directory = parent_fd(self.path); self.parent = self.directory.__enter__()
        try:
            flags = os.O_RDWR if self.writable else os.O_RDONLY
            flags |= os.O_NOFOLLOW|os.O_CLOEXEC|os.O_NONBLOCK
            created = False
            try:
                self.fd = os.open(self.path.name,flags,dir_fd=self.parent)
            except FileNotFoundError:
                w.require(self.writable and self.create_binding is not None, 'recovery requires an existing journal')
                validate_binding(self.create_binding)
                self.fd = os.open(self.path.name,flags|os.O_CREAT|os.O_EXCL,0o600,dir_fd=self.parent); created=True
            fcntl.flock(self.fd,(fcntl.LOCK_EX if self.writable else fcntl.LOCK_SH)|fcntl.LOCK_NB)
            self.identity = (os.fstat(self.fd).st_dev,os.fstat(self.fd).st_ino)
            self.dir_identity = (os.fstat(self.parent).st_dev,os.fstat(self.parent).st_ino)
            self.custody(os.fstat(self.fd).st_size)
            if created:
                header = {'format':FORMAT,'id':secrets.token_hex(32),'binding':self.create_binding}
                raw,_ = wrapped(header); self.write_all(raw)
                os.fsync(self.fd); os.fsync(self.parent)
            self.raw = self.read_all(); self.replay()
            if self.create_binding is not None:
                w.require(self.binding == self.create_binding, 'journal is bound to a different source')
            self.verify()
            if self.writable:
                # Re-certify complete recovered bytes before later native preparation.
                os.fsync(self.fd); os.fsync(self.parent); self.verify()
            return self
        except BaseException:
            self.close(); raise

    def close(self):
        if self.fd is not None: os.close(self.fd); self.fd=None
        if self.parent is not None: self.directory.__exit__(None,None,None); self.parent=None

    def __exit__(self,*_): self.close()

    def custody(self,size):
        check_time(self.deadline)
        opened=os.fstat(self.fd); named=os.stat(self.path.name,dir_fd=self.parent,follow_symlinks=False)
        directory=os.fstat(self.parent)
        with parent_fd(self.path) as current:
            actual=os.fstat(current)
            w.require((actual.st_dev,actual.st_ino) == self.dir_identity, 'journal parent path changed')
        w.require(stat.S_ISREG(opened.st_mode) and stat.S_ISREG(named.st_mode)
            and stat.S_IMODE(opened.st_mode) == 0o600 and stat.S_IMODE(named.st_mode) == 0o600
            and opened.st_uid in (0,os.geteuid()) and opened.st_nlink == 1
            and (opened.st_dev,opened.st_ino) == self.identity == (named.st_dev,named.st_ino)
            and opened.st_size == named.st_size == size and 0 <= size <= MAX_BYTES
            and stat.S_IMODE(directory.st_mode) == 0o700 and directory.st_uid in (0,os.geteuid()), 'journal custody changed')

    def read_all(self):
        info=os.fstat(self.fd); self.custody(info.st_size); os.lseek(self.fd,0,os.SEEK_SET); out=bytearray()
        while len(out) <= info.st_size:
            check_time(self.deadline); block=os.read(self.fd,min(65536,info.st_size+1-len(out)))
            if not block:break
            out+=block
        after=os.fstat(self.fd)
        w.require(len(out)==info.st_size and (info.st_mtime_ns,info.st_ctime_ns)==(after.st_mtime_ns,after.st_ctime_ns), 'journal changed during verification')
        self.custody(len(out)); return bytes(out)

    def replay(self):
        w.require(self.raw and self.raw.endswith(b'\n'), 'empty or torn journal; no automatic repair')
        lines=self.raw.splitlines(keepends=True)
        w.require(len(lines)<=MAX_EVENTS+1, 'journal event bound exceeded')
        header,self.head=decoded(lines[0])
        w.require(type(header) is dict and set(header)=={'format','id','binding'} and header['format']==FORMAT, 'wrong journal format')
        w.unhex(header['id'],32); self.id=header['id']; self.binding=validate_binding(header['binding'])
        self.entries={};self.events=0
        for line in lines[1:]:
            check_time(self.deadline); value,digest=decoded(line)
            w.require(type(value) is dict and set(value)=={'sequence','previous','entry'}, 'unexpected event fields')
            w.integer(value['sequence'],1,MAX_EVENTS)
            w.require(value['sequence']==self.events+1 and value['previous']==self.head, 'journal fork, gap or reordered event')
            entry=value['entry']; self.validate_entry(entry)
            self.entries[entry['plan']['key']]=entry;self.events+=1;self.head=digest

    def validate_entry(self,entry):
        w.require(type(entry) is dict and type(entry.get('plan')) is dict, 'invalid journal entry')
        operation_key=w.key(entry['plan'].get('key'));old=self.entries.get(operation_key)
        if old is None:
            w.require(len(self.entries)<MAX_KEYS and all(settled(e) for e in self.entries.values()), 'unsettled assignment blocks new intent')
        transition(old,entry,self.binding)

    def verify(self):
        w.require(not self.fenced, 'journal fenced; reopen verified recovery')
        w.require(self.read_all()==self.raw, 'retained journal bytes changed')

    def write_all(self,data):
        remaining=memoryview(data)
        while remaining:
            check_time(self.deadline); count=os.write(self.fd,remaining)
            w.require(count>0, 'short journal write');remaining=remaining[count:]

    def append(self,entry):
        w.require(self.writable and not self.fenced, 'journal does not permit writes');self.verify();self.validate_entry(entry)
        if self.entries.get(entry['plan']['key'])==entry:return
        reserve={'intent':3,'prepared':2,'dispatch_started':1,'tracking':1,'cancel_requested':1,'terminal':0,'cancelled_before_dispatch':0}[entry['state']]
        value={'sequence':self.events+1,'previous':self.head,'entry':entry};encoded,digest=wrapped(value)
        w.require(len(encoded)<=MAX_LINE and self.events+1+reserve<=MAX_EVENTS
            and len(self.raw)+len(encoded)+reserve*MAX_LINE<=MAX_BYTES, 'retention reserve exhausted before publication')
        # Keep every fallible construction before write; the root is published only after sync.
        following=copy.deepcopy(self.entries);following[entry['plan']['key']]=copy.deepcopy(entry)
        raw=self.raw+encoded
        try:
            os.lseek(self.fd,0,os.SEEK_END);self.write_all(encoded);os.fsync(self.fd);self.custody(len(raw))
            w.require(self.read_all()==raw, 'journal changed during publication')
            check_time(self.deadline)
            self.raw,self.head,self.events,self.entries=raw,digest,self.events+1,following
        except BaseException:
            self.fenced=True;raise

    def get(self,operation_key):
        self.verify();w.key(operation_key);w.require(operation_key in self.entries, 'assignment key is not in this journal')
        return copy.deepcopy(self.entries[operation_key])


def environment(control=False):
    w.require(os.environ.get('DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17')=='1'
        and all(not k.startswith('DFMCP_') or k in ALLOWED for k in os.environ), 'exact isolated workforce opt-in required')
    grant=os.environ.get('DFMCP_WORKFORCE_ALLOW_LABOR')
    w.require(grant is None or grant=='1', 'labor opt-in must be absent or exactly 1')
    if control:w.require(grant=='1','operator labor authority required')
    endpoint=os.environ.get('DFMCP_WORKFORCE_ENDPOINT','127.0.0.1:5000');w.address(endpoint)
    secret=os.environ.get('DFMCP_WORKFORCE_TOKEN','').encode();w.require(32<=len(secret)<=256,'workforce token required')
    return endpoint,secret


def match_source(client,binding,fresh):
    m=client.manifest
    w.require(client.endpoint==binding['endpoint'] and all(m[k]==binding[k] for k in ('df_version','dfhack_version'))
        and (m['generation']==binding['generation'] if fresh else m['generation']>=binding['generation']), 'journal/native source mismatch')


def authorize_now(client,binding):
    endpoint,_=environment(True);w.require(endpoint==binding['endpoint'],'operator endpoint changed')
    match_source(client,binding,True)


def accept_reply(journal,entry,result):
    encoded=result.get('effect_hex')
    if encoded is None:return False
    record=w.effect(w.unhex(encoded,1,w.MAX_EFFECT),entry['plan'])
    if record['phase'] in ('applied','refused','cancelled'):state='terminal'
    elif record['phase']=='prepared' and entry['state']=='intent':state='prepared'
    else:state='tracking'
    # Identical pending native evidence does not consume terminal retention space.
    next_entry={'plan':entry['plan'],'state':state,'effect_hex':encoded}
    if next_entry!=entry:journal.append(next_entry)
    return True


def reserve_response(plan):
    """Bound the entire single-effect response before allocating durable intent.

    Keys can contain JSON-escaped control text. Count their actual escaped size;
    each selected post-mask also reserves identity/flag/wrapper bytes. This is a
    UTF-8 byte bound, not a model-token estimate.
    """
    w.validate_plan(plan);before=w.capture(bytes.fromhex(plan['capture_hex']))
    texts={'source':{k:before[k] for k in ('generation','sequence','tick','folder','site')},
        'detail_name':before['details'][plan['detail_index']]['name'],
        'allowed_labor_keys':before['labor_keys'],'labor_keys':before['labor_keys']}
    reserve=8192+len(canonical(texts))+len(before['units'])*(2*len(before['labor_keys'])+256)
    w.require(reserve<=OUTPUT_LIMIT,'complete assignment response cannot fit; no native preparation or commit started')
    return reserve


def prepare(journal,client,plan):
    journal.verify();authorize_now(client,journal.binding);reserve_response(plan)
    old=journal.entries.get(plan['key'])
    if old is not None:
        w.require(old['plan']==plan,'key already binds a different assignment');return journal.get(plan['key'])
    entry={'plan':plan,'state':'intent','effect_hex':None}
    journal.append(entry)  # Before native preparation, including its replayable token allocation.
    authorize_now(client,journal.binding);journal.verify()
    result=client.call('PrepareAssignment',plan=plan);match_source(client,journal.binding,True)
    w.require(accept_reply(journal,entry,result),'native preparation returned no record')
    return journal.get(plan['key'])


def commit(journal,client,operation_key,confirmed):
    entry=journal.get(operation_key);reserve_response(entry['plan']);w.unhex(confirmed,32)
    w.require(confirmed==entry['plan']['plan_digest'],'reviewed plan digest required')
    w.require(entry['state']=='prepared','assignment is not dispatchable; query retained evidence')
    authorize_now(client,journal.binding)
    before=w.capture(bytes.fromhex(entry['plan']['capture_hex']))
    observed=client.call('ObserveWorkforce',unit_ids=[u['id'] for u in before['units']]);match_source(client,journal.binding,True)
    w.require(observed['capture_hex']==entry['plan']['capture_hex'],'workforce witness drifted; replan without dispatch')
    authorize_now(client,journal.binding)
    entry['state']='dispatch_started';journal.append(entry)  # Durable non-retryability BEFORE CommitAssignment.
    authorize_now(client,journal.binding);journal.verify()
    result=client.call('CommitAssignment',plan=entry['plan']);match_source(client,journal.binding,True)
    w.require(accept_reply(journal,entry,result),'commit reply absent; effect remains unresolved')
    return journal.get(operation_key)


def reconcile(journal,client,operation_key):
    entry=journal.get(operation_key)
    if settled(entry) or entry['state']=='prepared':return entry,False
    match_source(client,journal.binding,False)
    result=client.call('QueryAssignment',plan=entry['plan']);match_source(client,journal.binding,False)
    found=accept_reply(journal,entry,result)
    return journal.get(operation_key),not found


def cancel(journal,operation_key,client=None):
    environment(True);entry=journal.get(operation_key);reserve_response(entry['plan'])
    if settled(entry):return entry
    if entry['state'] in ('intent','prepared'):
        entry.update(state='cancelled_before_dispatch',effect_hex=None);journal.append(entry)
    else:
        w.require(client is not None, 'unresolved cancellation requires its native source')
        authorize_now(client,journal.binding)
        if entry['state']!='cancel_requested':
            entry['state']='cancel_requested';journal.append(entry)
        authorize_now(client,journal.binding);journal.verify()
        result=client.call('CancelAssignment',plan=entry['plan']);match_source(client,journal.binding,True)
        # Native cancellation retires a Prepared token. It cannot undo Applied or
        # repair Unknown. A dispatched marker can never regain eligibility.
        w.require(accept_reply(journal,entry,result),'cancel reply absent; assignment remains unresolved')
    return journal.get(operation_key)


def project(entry):
    plan=entry['plan'];before=w.capture(bytes.fromhex(plan['capture_hex']));detail=before['details'][plan['detail_index']]
    result={'key':plan['key'],'plan_digest':plan['plan_digest'],'state':entry['state'],
        'source':{k:before[k] for k in ('generation','sequence','tick','folder','site')},
        'detail_index':plan['detail_index'],'detail_name':detail['name'],'assigned':plan['assigned'],
        'unit_ids':[u['id'] for u in before['units']], 'allowed_labor_keys':[k for i,k in enumerate(before['labor_keys']) if detail['labors'][i]],
        'current_state_proven':False,'job_completion_proven':False}
    if entry['effect_hex']:
        record=w.effect(bytes.fromhex(entry['effect_hex']),plan)
        result['native_evidence']={k:v for k,v in record.items() if k!='post_units'}
        result['native_evidence']['post_units']=[{**{k:v for k,v in u.items() if k!='labors'},
            'labor_mask_hex':bytes(u['labors']).hex()} for u in record['post_units']]
        result['native_evidence']['labor_keys']=before['labor_keys']
    return result


def inspect(journal,key=None,limit=4,after=None,head=None):
    journal.verify()
    if key is not None:return {'effect':project(journal.get(key)),'journal_head':journal.head,'native_contacted':False}
    w.integer(limit,1,8)
    if after is not None:
        w.key(after);w.require(head==journal.head and after in journal.entries,'continuation head/key mismatch')
    elif head is not None:w.require(head==journal.head,'journal head drift')
    keys=[k for k in sorted(journal.entries) if after is None or k>after];page=keys[:limit]
    # Discovery stays compact; exact per-key inspection returns labor detail.
    rows=[{'key':k,'state':journal.entries[k]['state'],'plan_digest':journal.entries[k]['plan']['plan_digest']} for k in page]
    return {'records':rows,'journal_head':journal.head,'journal_id':journal.id,'total_records':len(journal.entries),
        'unsettled_records':sum(not settled(e) for e in journal.entries.values()),
        'next_after':page[-1] if len(keys)>len(page) else None,'native_contacted':False}


def main(argv=None):
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('operation',choices=('observe','prepare','commit','query','cancel','inspect'))
    p.add_argument('--journal',type=Path);p.add_argument('--key');p.add_argument('--units')
    p.add_argument('--detail',type=int);p.add_argument('--assigned',choices=('yes','no'))
    p.add_argument('--world-folder');p.add_argument('--site-id',type=int);p.add_argument('--confirm-plan')
    p.add_argument('--limit',type=int,default=4);p.add_argument('--after-key');p.add_argument('--head')
    p.add_argument('--timeout-ms',type=int,default=10000);a=p.parse_args(argv)
    try:
        w.integer(a.timeout_ms,1,60000);deadline=time.monotonic()+a.timeout_ms/1000
        w.require(a.operation=='observe' or a.journal is not None,'--journal required')
        w.require(a.operation in ('observe','prepare') or a.units is None,'--units is observe/prepare only')
        w.require(a.operation=='prepare' or all(v is None for v in (a.detail,a.assigned,a.world_folder,a.site_id)), 'prepare-only argument supplied')
        w.require(a.operation=='commit' or a.confirm_plan is None,'--confirm-plan is commit only')
        w.require(a.operation=='inspect' or (a.after_key is None and a.head is None and a.limit==4), 'pagination is inspect only')
        if a.operation in ('prepare','commit','query','cancel'):w.key(a.key)
        if a.operation in ('observe','prepare'):
            w.require(type(a.units) is str and len(a.units)<=351,'--units requires bounded comma-separated IDs')
            selected=w.ids([int(n) for n in a.units.split(',')])
            if a.operation=='prepare':
                w.integer(a.detail,0,63);w.require(a.assigned is not None,'--assigned required')
                w.text(a.world_folder,512);w.integer(a.site_id,0,2**31-1)
            endpoint,secret=environment(a.operation=='prepare')
            with w.Client(endpoint,secret,deadline) as client:
                observed=client.call('ObserveWorkforce',unit_ids=selected);raw=bytes.fromhex(observed['capture_hex']);before=w.capture(raw)
                if a.operation=='observe':
                    result={'observation':before,'witness':hashlib.sha256(raw).hexdigest(),'native_contacted':True}
                else:
                    w.require(before['folder']==a.world_folder and before['site']==a.site_id,'selected fortress differs from native source')
                    plan=w.make_plan(a.key,a.detail,a.assigned=='yes',raw)
                    binding={**client.manifest,'endpoint':endpoint,'folder':before['folder'],'site':before['site']}
                    with Journal(a.journal,deadline,True,binding) as journal:
                        result={'effect':project(prepare(journal,client,plan)),'journal_head':journal.head}
        elif a.operation=='inspect':
            with Journal(a.journal,deadline) as journal:
                result=inspect(journal,a.key,a.limit,a.after_key,a.head)
        else:
            environment(a.operation in ('commit','cancel'))
            with Journal(a.journal,deadline,True) as journal:
                if a.operation=='cancel':
                    entry=journal.get(a.key);native=entry['state'] not in ('intent','prepared','terminal','cancelled_before_dispatch')
                    if native:
                        endpoint,secret=environment(True)
                        w.require(endpoint==journal.binding['endpoint'],'configured endpoint differs from journal')
                        with w.Client(endpoint,secret,deadline) as client:entry=cancel(journal,a.key,client)
                    else:entry=cancel(journal,a.key)
                    result={'effect':project(entry),'native_contacted':native,'membership_undone':False}
                else:
                    entry=journal.get(a.key)
                    if a.operation=='query' and (settled(entry) or entry['state']=='prepared'):
                        result={'effect':project(entry),'native_contacted':False}
                    else:
                        endpoint,secret=environment(a.operation=='commit')
                        w.require(endpoint==journal.binding['endpoint'],'configured endpoint differs from journal')
                        with w.Client(endpoint,secret,deadline) as client:
                            if a.operation=='commit':entry=commit(journal,client,a.key,a.confirm_plan);absent=False
                            else:entry,absent=reconcile(journal,client,a.key)
                        result={'effect':project(entry),'native_contacted':True,'native_record_absent':absent,
                                'absence_proves_nonapplication':False}
        encoded=canonical({'ok':True,'profile':'workforce/1.17','runtime_admitted':False,'result':result})
        w.require(len(encoded)<=OUTPUT_LIMIT,'response exceeded bound; recover retained evidence, never replay commit')
        print(encoded.decode());return 0
    except (OSError,ValueError,TypeError,KeyError,RecursionError) as error:
        print(json.dumps({'ok':False,'profile':'workforce/1.17','runtime_admitted':False,'effect_status':'unknown',
            'error_class':type(error).__name__,'detail':str(error) if isinstance(error,w.Rejected) else
            'I/O, custody or decoding refused; inspect retained journal before further control'}));return 2

if __name__=='__main__':raise SystemExit(main())
