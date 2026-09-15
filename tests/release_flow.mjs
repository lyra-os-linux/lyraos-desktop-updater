import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

function fixture(locale = 'pt-BR', saved = null) {
  const nodes = new Map(), calls = [], timers = new Map();
  let sequence = 0;
  function node(id) {
    if (!nodes.has(id)) nodes.set(id, {style: {}, value: id === '#filter' ? 'all' : '', hidden: false,
      listeners: {}, addEventListener(kind, fn) {this.listeners[kind] = fn;}, focus() {},
      querySelector: node});
    return nodes.get(id);
  }
  const state = {reply: null, confirm: false, storage: saved};
  const invoke = async (method, args) => {
    if (method === 'color_scheme') return 'dark';
    if (method === 'layout_preview_enabled') return false;
    assert.equal(method, 'service_request');
    calls.push(args.request);
    return state.reply ? state.reply(args.request) : {kind: 'Rejected', error_code: 'RELEASE_NOT_AVAILABLE'};
  };
  const context = vm.createContext({
    window: {__TAURI__: {core: {invoke}}, addEventListener() {}, confirm: () => state.confirm},
    document: {documentElement: {dataset: {}}, querySelector: node, querySelectorAll: () => [],
      createElement: () => ({set textContent(value) {this.innerHTML = String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');}})},
    navigator: {language: locale}, crypto: {randomUUID: () => 'request-id'},
    localStorage: {getItem: () => state.storage, setItem: (_key, value) => {state.storage=value;}, removeItem: () => {state.storage=null;}},
    setInterval: fn => {timers.set(++sequence, fn); return sequence;}, clearInterval: id => timers.delete(id),
  });
  for (const name of ['i18n.js', 'errors.js', 'app.js']) vm.runInContext(fs.readFileSync(new URL(`../ui/${name}`, import.meta.url), 'utf8'), context);
  return {node, calls, state, context, run: source => vm.runInContext(source, context)};
}
const drain = async () => {for(let n=0;n<12;n++) await Promise.resolve();};
function plan() {
  const plan = {operation:'ReleaseUpgrade',source:{version:'1.1'},target:{version:'1.2'},
    required_bytes:1024,reboot_required:true,repositories:[{alias:'repo-lyra'}],
    installed_packages:[],package_changes:[{name:'<script>unsafe</script>',architecture:'noarch',action:'Upgrade',current_version:'1-1',proposed_version:'2-1',current_vendor:'Lyra',proposed_vendor:'Lyra'}]};
  return {kind:'Plan',operation_id:'test-operation',plan_sha256:'a'.repeat(64),plan,planned:{plan}};
}

test('opening, checking a release and planning never start an administrative operation', async () => {
  const f=fixture();await drain();
  assert.deepEqual(f.calls.map(r=>r.kind),['CheckRelease']);
  f.state.reply = request => request.kind === 'CheckRelease'
    ? {kind:'ReleaseOffer',manifest_sha256:'a'.repeat(64),manifest:{target:{version:'1.2',build_id:'qualified'}},cached:false}
    : plan();
  await f.run('checkRelease()');await f.run('planRelease()');
  assert(!f.calls.some(r=>r.kind==='Start'));
  assert(f.calls.every(r=>r.protocol_version===3));
  assert(f.node('#confirm').disabled);
  assert(f.node('#package-plan').innerHTML.includes('&lt;script&gt;'));
  assert(!f.node('#package-plan').innerHTML.includes('<script>'));
  await f.run('confirmUpdate()');
  assert(!f.calls.some(r=>r.kind==='Start'));
  f.node('#backup-ack').checked=true;
  f.state.reply=()=>{throw new Error('AUTHORIZATION');};
  await f.run('confirmUpdate()');
  assert.equal(f.calls.filter(r=>r.kind==='Start').length,1);
  assert.equal(f.state.storage,null);
  assert.equal(f.node('#confirm').hidden,false);
});

test('declining rollback never sends Start or recovery; approving sends only rollback', async () => {
  const f=fixture();await drain();f.calls.length=0;
  await f.run('recover("Rollback")');assert.equal(f.calls.length,0);
  f.state.confirm=true;f.state.reply=()=>({kind:'Accepted'});
  await f.run('recover("Rollback")');
  assert.deepEqual(f.calls.map(r=>r.kind),['AcknowledgeRecovery']);
  assert.equal(f.calls[0].recovery_action,'Rollback');
});

test('a missing resumed operation is forgotten without authentication or a polling loop', async () => {
  const f=fixture('en-US',JSON.stringify({operationId:'missing',planHash:'a'}));
  f.state.reply=()=>({kind:'Rejected',error_code:'OPERATION_NOT_FOUND'});await drain();
  assert.equal(f.state.storage,null);
  assert.equal(f.run('state.operationId'),null);
  assert.equal(f.run('state.poll'),null);
  assert(!f.calls.some(r=>r.kind==='Start'));
});

test('a signed cached offer is display-only and all new labels exist in three languages', async () => {
  for(const locale of ['en-US','pt-BR','es-ES']) {
    const f=fixture(locale);await drain();
    f.state.reply=()=>({kind:'ReleaseOffer',manifest_sha256:'a'.repeat(64),manifest:{target:{version:'1.2',build_id:'test'}},cached:true});
    await f.run('checkRelease()');assert.equal(f.node('#plan-release').disabled,true);
    for(const key of ['check_release','plan_release','snapshot_scope','backup_ack','release_cached','configuration_artifacts']) {
      assert.notEqual(f.run(`t(${JSON.stringify(key)})`),key);
    }
  }
});

test('preflight blockers explain the condition in every supported language without admin actions', async () => {
  for (const locale of ['pt-BR','en-US','es-ES']) {
    const f=fixture(locale);await drain();f.calls.length=0;
    f.state.reply=()=>({kind:'PreflightBlocked',blockers:['HOME_NOT_ISOLATED',{'UNAUTHORIZED_REMOVAL':{package:'personal-app'}}]});
    await f.run('check()');
    assert.equal(f.run('state.currentState'),'Blocked');
    assert(f.run('errorMessage({preflight:"readable reason"})')==='readable reason');
    assert.notEqual(f.run('errorMessage("HOME_NOT_ISOLATED")'),'error_HOME_NOT_ISOLATED');
    assert(!f.calls.some(request=>request.kind==='Start'));
  }
});
