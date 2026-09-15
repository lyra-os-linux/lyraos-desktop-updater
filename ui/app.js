const invoke = window.__TAURI__.core.invoke;
const catalogs = window.LYRA_UPGRADE_CATALOGS;
const locale = (() => { const value=(navigator.language||"en-US").toLowerCase(); return value.startsWith("pt")?"pt-BR":value.startsWith("es")?"es-ES":"en-US"; })();
const t = key => catalogs[locale][key] || catalogs["en-US"][key] || key;
document.documentElement.lang=locale;
document.querySelectorAll("[data-i18n]").forEach(node=>node.textContent=t(node.dataset.i18n));
document.querySelectorAll("[data-i18n-placeholder]").forEach(node=>node.placeholder=t(node.dataset.i18nPlaceholder));

// Paint the window in the desktop's appearance. Failing quietly is correct:
// styles.css then falls back to prefers-color-scheme.
invoke("color_scheme").then(scheme=>{ if(scheme==="light"||scheme==="dark") document.documentElement.dataset.scheme=scheme; }).catch(()=>{});

const state={operationId:null,planHash:null,planned:null,events:[],lastSequence:0,tab:"events",poll:null,currentState:"Checking",snapshotNumber:null,releaseOffer:null,report:null,reviewedPlan:null,polling:false};
const persistedOperationKey="lyra-upgrade-active-operation-v1";
const phases=["Checking","Preflight","Downloading","Snapshotting","Applying","AwaitingReboot","VerifyingBoot","Completed"];
const progress={Checking:5,Preflight:12,Planned:18,AwaitingConfirmation:18,Downloading:42,Snapshotting:52,Applying:76,ReadyToReboot:85,ApplyingOffline:90,AwaitingReboot:94,VerifyingBoot:97,Completed:100,Failed:100,NeedsRecovery:100,Blocked:12};
const titleKeys={Checking:"checking",Preflight:"checking",Blocked:"blocked",Planned:"planned",AwaitingConfirmation:"planned",Downloading:"downloading",Snapshotting:"snapshotting",Applying:"applying",ApplyingOffline:"applying",ReadyToReboot:"awaiting_reboot",AwaitingReboot:"awaiting_reboot",VerifyingBoot:"verifying",Completed:"completed",Failed:"failed",NeedsRecovery:"needs_recovery"};

function request(kind,extra={}){return invoke("service_request",{request:{protocol_version:3,request_id:crypto.randomUUID(),kind,...extra}});}
function errorMessage(error){if(error?.preflight)return error.preflight;const code=String(error?.message||error||"UNKNOWN").replace(/^Error:\s*/,"");return t(`error_${code}`)===`error_${code}`?t("error_UNKNOWN"):t(`error_${code}`);}
function rememberOperation(){localStorage.setItem(persistedOperationKey,JSON.stringify({operationId:state.operationId,planHash:state.planHash}));}
function forgetOperation(){localStorage.removeItem(persistedOperationKey);}
function setError(message){document.querySelector("#error").textContent=message||"";}
function updateState(name,recovered=false){
  state.currentState=name;
  const percent=progress[name]??0;
  document.querySelector("#progress-bar").style.width=`${percent}%`;
  document.querySelector("#progress-label").textContent=`${percent}%`;
  const key=name==="Completed"&&recovered?"recovered":titleKeys[name]; if(key) document.querySelector("#operation-title").textContent=t(key);
  document.querySelector("#state-pill").textContent=t(key||"idle");
  document.querySelector("#operation-message").textContent=["Applying","ApplyingOffline","Snapshotting"].includes(name)?t("no_cancel"):t(key==="planned"?"planned_help":"checking_help");
  document.querySelector("#restart").hidden=!["ReadyToReboot","AwaitingReboot"].includes(name);
  document.querySelector("#rollback").hidden=name!=="NeedsRecovery"||!state.snapshotNumber;
  document.querySelector("#keep-current").hidden=name!=="NeedsRecovery";
  renderPhases(name);
}
function renderPhases(active){document.querySelector("#phases").innerHTML=phases.map(name=>`<li class="${name===active?"active":(progress[name]??0)<(progress[active]??0)?"done":""}"><span></span>${escapeHtml(t(`phase_${name}`))}</li>`).join("");}
function addEvents(items){for(const event of items||[]){if(event.sequence===0){state.events=state.events.filter(old=>old.sequence!==0||old.message_id!==event.message_id);}else if(state.events.some(old=>old.sequence===event.sequence))continue;state.events.push(event);state.lastSequence=Math.max(state.lastSequence,event.sequence);}state.events.sort((a,b)=>a.sequence-b.sequence);renderDetails();}
function renderDetails(){
  const query=document.querySelector("#search").value.toLowerCase(); const filter=document.querySelector("#filter").value;
  const visible=state.events.filter(event=>(filter==="all"||event.level?.toLowerCase()===filter)&&JSON.stringify(event).toLowerCase().includes(query));
  document.querySelector("#event-list").innerHTML=visible.filter(e=>!e.technical).map(e=>`<article class="event ${e.level?.toLowerCase()}"><time>${escapeHtml(e.occurred_at||"")}</time><span>${escapeHtml(t(e.message_id)||e.message_id)}</span></article>`).join("");
  document.querySelector("#console").textContent=visible.filter(e=>e.technical).map(e=>e.technical.text+(e.technical.truncated?" …":"")).join("\n");
}
function escapeHtml(value){const node=document.createElement("span");node.textContent=value;return node.innerHTML;}
function showPlan(response) {
  if(response.kind==="PreflightBlocked"){
    const descriptions=response.blockers.map(blocker=>{
      const code=typeof blocker==="string"?blocker:Object.keys(blocker)[0];
      const details=typeof blocker==="string"?null:blocker[code];
      return errorMessage(code)+(details?.package?` (${details.package})`:details?.alias?` (${details.alias})`:"");
    });
    const error=new Error("PREFLIGHT_BLOCKED");error.preflight=descriptions.join("\n");throw error;
  }
  if(response.kind!=="Plan") throw new Error(response.error_code||"PREFLIGHT_BLOCKED");
  state.operationId=response.operation_id; state.planHash=response.plan_sha256; state.planned=response.planned;
  state.reviewedPlan=response.plan;
  const plan=response.plan, release=plan.operation==="ReleaseUpgrade";
  document.querySelector("#plan-summary").hidden=false;
  document.querySelector("#plan-summary").textContent=`${plan.source.version}${plan.target?` → ${plan.target.version}`:""} · ${plan.package_changes.length} ${t("packages")} · ${t("space")}: ${formatBytes(plan.required_bytes)} · ${plan.reboot_required?t("reboot_yes"):t("reboot_no")}`;
  document.querySelector("#plan-review").hidden=false;
  document.querySelector("#backup-confirmation").hidden=!release;
  document.querySelector("#backup-ack").checked=false;
  document.querySelector("#confirm").disabled=release;
  document.querySelector("#package-plan").innerHTML=plan.package_changes.map(p=>`<p><strong>${escapeHtml(p.name)}</strong> (${escapeHtml(p.architecture)}) · ${escapeHtml(t(`action_${p.action}`))}<br>${escapeHtml(p.current_version||"—")} → ${escapeHtml(p.proposed_version||"—")}<br>${escapeHtml(p.current_vendor||"—")} → ${escapeHtml(p.proposed_vendor||"—")}</p>`).join("")||escapeHtml(t("no_package_changes"));
  document.querySelector("#repository-plan").textContent=plan.repositories.map(r=>r.alias).join(" · ")+(release?`\n${t("third_party_disabled")}`:"");
  document.querySelector("#confirm").hidden=false;
  document.querySelector("#check").hidden=true;
  document.querySelector("#plan-release").hidden=true;
  document.querySelector("#check-release").hidden=true;
  updateState("AwaitingConfirmation");
}
async function check(){state.events=[];state.lastSequence=0;state.report=null;renderReport();setError("");updateState("Checking");try{showPlan(await request("PlanUpdate"));}catch(error){setError(errorMessage(error));updateState("Blocked");}}
async function checkRelease(){
  if(state.planned||state.poll||state.operationId&&!(["Completed","Failed"].includes(state.currentState)))return;
  const button=document.querySelector("#check-release"),status=document.querySelector("#release-status");
  button.disabled=true; state.releaseOffer=null; document.querySelector("#plan-release").hidden=true; status.textContent=t("release_checking");
  try {
    const response=await request("CheckRelease");
    if(response.kind!=="ReleaseOffer")throw new Error(response.error_code||"MANIFEST_INVALID");
    state.releaseOffer=response;
    status.textContent=`${t("release_available")}: ${response.manifest.target.version} (${response.manifest.target.build_id})${response.cached?` · ${t("release_cached")}`:""}`;
    document.querySelector("#plan-release").hidden=false;
    document.querySelector("#plan-release").disabled=response.cached===true;
  } catch(error) { status.textContent=errorMessage(error); }
  finally { button.disabled=false; }
}
async function planRelease(){
  if(!state.releaseOffer)return;
  setError("");updateState("Checking");
  try { showPlan(await request("PlanReleaseUpgrade",{manifest_sha256:state.releaseOffer.manifest_sha256})); }
  catch(error){setError(errorMessage(error));updateState("Blocked");}
}
async function confirmUpdate(){
  const button=document.querySelector("#confirm");button.disabled=true;setError("");
  try{
    if(!state.planned)throw new Error("PLAN_NOT_AVAILABLE");
    if(state.planned.plan.operation==="ReleaseUpgrade"&&!document.querySelector("#backup-ack").checked)throw new Error("BACKUP_ACK_REQUIRED");
    const response=await request("Start",{operation_id:state.operationId,plan_sha256:state.planHash,confirmed:true,planned:state.planned});
    if(response.kind!=="Accepted")throw new Error(response.error_code||"INVALID_RESPONSE");
    button.hidden=true;state.planned=null;rememberOperation();startPolling();
  }catch(error){setError(errorMessage(error));document.querySelector("#check").hidden=false;}
  finally{button.disabled=state.planned?.plan.operation==="ReleaseUpgrade"&&!document.querySelector("#backup-ack").checked;}
}
async function poll(){
  if(!state.operationId||state.polling)return;
  state.polling=true;
  try{
    const response=await request("Status",{operation_id:state.operationId,after_sequence:state.lastSequence});
    if(response.kind==="Rejected"){
      if(response.error_code==="OPERATION_NOT_FOUND"){
        forgetOperation();state.operationId=null;clearInterval(state.poll);state.poll=null;
        document.querySelector("#check").hidden=false;document.querySelector("#check-release").hidden=false;
      }
      throw new Error(response.error_code);
    }
    if(response.kind==="Status"){
      state.report=response.report||null;renderReport();addEvents(response.events);
      state.snapshotNumber=response.snapshot_number||null;updateState(response.state,response.recovered===true);
      setError(response.error_code?errorMessage(response.error_code):"");
      const terminal=["Completed","Failed"].includes(response.state);
      document.querySelector("#check").hidden=!terminal;document.querySelector("#check-release").hidden=!terminal;
      document.querySelector("#confirm").hidden=true;
      if(terminal||response.state==="NeedsRecovery"){clearInterval(state.poll);state.poll=null;}
      if(terminal)forgetOperation();
    }
  }catch(error){setError(errorMessage(error));}
  finally{state.polling=false;}
}
function startPolling(){if(state.poll)clearInterval(state.poll);poll();state.poll=setInterval(poll,1000);}
async function resumeOperation(){
  let saved;
  try{saved=JSON.parse(localStorage.getItem(persistedOperationKey)||"null");}catch(_){forgetOperation();return;}
  if(!saved?.operationId)return;
  state.operationId=saved.operationId;
  state.planHash=saved.planHash||null;
  await poll();
  if(state.operationId&&!["Completed","Failed","NeedsRecovery"].includes(state.currentState)&&state.poll===null)startPolling();
}
function formatBytes(bytes){const units=["B","KiB","MiB","GiB"];let value=bytes,index=0;while(value>=1024&&index<units.length-1){value/=1024;index++;}return `${value.toFixed(index?1:0)} ${units[index]}`;}
function toggleDetails(){const details=document.querySelector("#details"),button=document.querySelector("#details-toggle"),open=details.hidden;details.hidden=!open;button.ariaExpanded=String(open);button.textContent=t(open?"hide_details":"show_details");if(open)document.querySelector("#events-tab").focus();}
function chooseTab(tab){state.tab=tab;document.querySelector("#event-list").hidden=tab!=="events";document.querySelector("#console").hidden=tab!=="console";document.querySelector("#events-tab").ariaSelected=String(tab==="events");document.querySelector("#console-tab").ariaSelected=String(tab==="console");}
function navigateTabs(event){if(!["ArrowLeft","ArrowRight"].includes(event.key))return;event.preventDefault();const tab=state.tab==="events"?"console":"events";chooseTab(tab);document.querySelector(`#${tab}-tab`).focus();}
async function copyVisible(){const text=state.tab==="console"?document.querySelector("#console").textContent:document.querySelector("#event-list").innerText;await navigator.clipboard.writeText(text);}
function exportVisible(){const data=JSON.stringify({schema:1,operation_id:state.operationId,events:state.events,plan:state.reviewedPlan,report:state.report},null,2);const url=URL.createObjectURL(new Blob([data],{type:"application/json"}));const link=document.createElement("a");link.href=url;link.download=`lyra-upgrade-${state.operationId||"diagnostic"}.json`;link.click();URL.revokeObjectURL(url);}
function renderReport(){
  const node=document.querySelector("#post-report");node.hidden=!state.report;
  if(state.report) node.textContent=`${state.report.installed_packages.length} ${t("packages")} · ${t("configuration_artifacts")}: ${state.report.configuration_artifacts.length}\n${state.report.configuration_artifacts.join("\n")}`;
}
async function recover(action){if(action==="Rollback"&&!window.confirm(t("rollback_confirm")))return;try{const response=await request("AcknowledgeRecovery",{operation_id:state.operationId,recovery_action:action});if(response.kind==="Rejected")throw new Error(response.error_code);await poll();}catch(error){setError(errorMessage(error));}}
async function restart(){try{await invoke("reboot_system");}catch(error){setError(errorMessage(error));}}
function writeInProgress(){return ["Snapshotting","Applying","ReadyToReboot","ApplyingOffline"].includes(state.currentState);}
window.addEventListener("beforeunload",event=>{if(writeInProgress()){event.preventDefault();event.returnValue="";}});
document.querySelector("#check").addEventListener("click",check);document.querySelector("#confirm").addEventListener("click",confirmUpdate);document.querySelector("#restart").addEventListener("click",restart);document.querySelector("#rollback").addEventListener("click",()=>recover("Rollback"));document.querySelector("#keep-current").addEventListener("click",()=>recover("KeepCurrent"));document.querySelector("#details-toggle").addEventListener("click",toggleDetails);document.querySelector("#events-tab").addEventListener("click",()=>chooseTab("events"));document.querySelector("#console-tab").addEventListener("click",()=>chooseTab("console"));document.querySelector(".tabs").addEventListener("keydown",navigateTabs);document.querySelector("#copy").addEventListener("click",copyVisible);document.querySelector("#export").addEventListener("click",exportVisible);document.querySelector("#filter").addEventListener("change",renderDetails);document.querySelector("#search").addEventListener("input",renderDetails);renderPhases("Checking");

function previewState(name) {
  const planned=name==="AwaitingConfirmation";
  document.querySelector("#plan-summary").hidden=!planned;
  document.querySelector("#plan-summary").textContent=`18 ${t("packages")} · ${t("space")}: 642.0 MiB`;
  document.querySelector("#check").hidden=name!=="Checking";
  document.querySelector("#confirm").hidden=!planned;
  setError(name==="Failed"?t("preview_failed"):name==="NeedsRecovery"?t("preview_recovery"):"");
  updateState(name);
}

async function enableLayoutPreview() {
  if (!await invoke("layout_preview_enabled")) return false;
  const toolbar=document.querySelector("#preview-toolbar");
  toolbar.hidden=false;
  document.querySelector("#check-release").disabled=true;
  document.querySelector("#plan-release").disabled=true;
  document.querySelector("#check").disabled=true;
  document.querySelector("#confirm").disabled=true;
  addEvents([
    {sequence:1,occurred_at:"14:32:01",level:"Info",message_id:"Verificação do sistema concluída"},
    {sequence:2,occurred_at:"14:32:03",level:"Warning",message_id:"Um pacote será mantido na versão atual"},
    {sequence:3,occurred_at:"14:32:04",level:"Info",message_id:"zypper",technical:{text:"Retrieving repository 'Lyra Updates' metadata…\nReading installed packages…\n18 packages to upgrade."}}
  ]);
  toolbar.querySelector("#preview-state").addEventListener("change",event=>previewState(event.target.value));
  previewState("Checking");
  return true;
}

document.querySelector("#check-release").addEventListener("click",checkRelease);
document.querySelector("#plan-release").addEventListener("click",planRelease);
document.querySelector("#backup-ack").addEventListener("change",()=>{document.querySelector("#confirm").disabled=!document.querySelector("#backup-ack").checked;});
enableLayoutPreview().then(async enabled=>{if(!enabled){await resumeOperation();if(!state.operationId){checkRelease();setInterval(()=>{if(!state.operationId)checkRelease();},6*60*60*1000);}}});
