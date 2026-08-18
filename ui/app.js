const invoke = window.__TAURI__.core.invoke;
const catalogs = window.LYRA_UPGRADE_CATALOGS;
const locale = (() => { const value=(navigator.language||"en-US").toLowerCase(); return value.startsWith("pt")?"pt-BR":value.startsWith("es")?"es-ES":"en-US"; })();
const t = key => catalogs[locale][key] || catalogs["en-US"][key] || key;
document.documentElement.lang=locale;
document.querySelectorAll("[data-i18n]").forEach(node=>node.textContent=t(node.dataset.i18n));
document.querySelectorAll("[data-i18n-placeholder]").forEach(node=>node.placeholder=t(node.dataset.i18nPlaceholder));

const state={operationId:null,planHash:null,planned:null,events:[],lastSequence:0,tab:"events",poll:null,currentState:"Checking",snapshotNumber:null};
const persistedOperationKey="lyra-upgrade-active-operation-v1";
const phases=["Checking","Preflight","Downloading","Snapshotting","Applying","AwaitingReboot","VerifyingBoot","Completed"];
const progress={Checking:5,Preflight:12,Planned:18,AwaitingConfirmation:18,Downloading:42,Snapshotting:52,Applying:76,ReadyToReboot:85,ApplyingOffline:90,AwaitingReboot:94,VerifyingBoot:97,Completed:100,Failed:100,NeedsRecovery:100,Blocked:12};
const titleKeys={Checking:"checking",Preflight:"checking",Blocked:"blocked",Planned:"planned",AwaitingConfirmation:"planned",Downloading:"downloading",Snapshotting:"snapshotting",Applying:"applying",ApplyingOffline:"applying",ReadyToReboot:"awaiting_reboot",AwaitingReboot:"awaiting_reboot",VerifyingBoot:"verifying",Completed:"completed",Failed:"failed",NeedsRecovery:"needs_recovery"};

function request(kind,extra={}){return invoke("service_request",{request:{protocol_version:2,request_id:crypto.randomUUID(),kind,...extra}});}
function errorMessage(error){const code=String(error?.message||error||"UNKNOWN").replace(/^Error:\s*/,"");return t(`error_${code}`)===`error_${code}`?t("error_UNKNOWN"):t(`error_${code}`);}
function rememberOperation(){localStorage.setItem(persistedOperationKey,JSON.stringify({operationId:state.operationId,planHash:state.planHash}));}
function forgetOperation(){localStorage.removeItem(persistedOperationKey);}
function setError(message){document.querySelector("#error").textContent=message||"";}
function updateState(name){
  state.currentState=name;
  const percent=progress[name]??0;
  document.querySelector("#progress-bar").style.width=`${percent}%`;
  document.querySelector("#progress-label").textContent=`${percent}%`;
  const key=titleKeys[name]; if(key) document.querySelector("#operation-title").textContent=t(key);
  document.querySelector("#state-pill").textContent=t(key||"idle");
  document.querySelector("#operation-message").textContent=["Applying","ApplyingOffline","Snapshotting"].includes(name)?t("no_cancel"):t(key==="planned"?"planned_help":"checking_help");
  document.querySelector("#restart").hidden=!["ReadyToReboot","AwaitingReboot"].includes(name);
  document.querySelector("#rollback").hidden=name!=="NeedsRecovery"||!state.snapshotNumber;
  document.querySelector("#keep-current").hidden=name!=="NeedsRecovery";
  renderPhases(name);
}
function renderPhases(active){document.querySelector("#phases").innerHTML=phases.map(name=>`<li class="${name===active?"active":(progress[name]??0)<(progress[active]??0)?"done":""}"><span></span>${escapeHtml(t(`phase_${name}`))}</li>`).join("");}
function addEvents(items){for(const event of items||[]){if(state.events.some(old=>old.sequence===event.sequence))continue;state.events.push(event);state.lastSequence=Math.max(state.lastSequence,event.sequence);}state.events.sort((a,b)=>a.sequence-b.sequence);renderDetails();}
function renderDetails(){
  const query=document.querySelector("#search").value.toLowerCase(); const filter=document.querySelector("#filter").value;
  const visible=state.events.filter(event=>(filter==="all"||event.level?.toLowerCase()===filter)&&JSON.stringify(event).toLowerCase().includes(query));
  document.querySelector("#event-list").innerHTML=visible.filter(e=>!e.technical).map(e=>`<article class="event ${e.level?.toLowerCase()}"><time>${escapeHtml(e.occurred_at||"")}</time><span>${escapeHtml(t(e.message_id)||e.message_id)}</span></article>`).join("");
  document.querySelector("#console").textContent=visible.filter(e=>e.technical).map(e=>e.technical.text+(e.technical.truncated?" …":"")).join("\n");
}
function escapeHtml(value){const node=document.createElement("span");node.textContent=value;return node.innerHTML;}
async function check(){setError("");updateState("Checking");try{const operationId=crypto.randomUUID();const response=await invoke("plan_update",{requestId:crypto.randomUUID(),operationId});if(response.kind!=="Plan")throw new Error(response.error_code||"PREFLIGHT_BLOCKED");state.operationId=response.operation_id;state.planHash=response.plan_sha256;state.planned=response.planned;const plan=response.plan;const reboot=plan.reboot_required?t("reboot_yes"):t("reboot_no");document.querySelector("#plan-summary").hidden=false;document.querySelector("#plan-summary").textContent=`${plan.source.version} (${plan.source.build_id}) · ${plan.package_changes.length} ${t("packages")} · ${t("space")}: ${formatBytes(plan.required_bytes)} · ${reboot}`;document.querySelector("#confirm").hidden=false;document.querySelector("#check").hidden=true;updateState("AwaitingConfirmation");}catch(error){setError(errorMessage(error));updateState("Blocked");}}
async function confirm(){document.querySelector("#confirm").hidden=true;try{if(!state.planned)throw new Error("PLAN_NOT_AVAILABLE");const response=await request("Start",{operation_id:state.operationId,plan_sha256:state.planHash,confirmed:true,planned:state.planned});if(response.kind==="Rejected")throw new Error(response.error_code);state.planned=null;rememberOperation();startPolling();}catch(error){setError(errorMessage(error));updateState("Failed");}}
async function poll(){if(!state.operationId)return;try{const response=await request("Status",{operation_id:state.operationId,after_sequence:state.lastSequence});if(response.kind==="Rejected"){if(response.error_code==="OPERATION_NOT_FOUND")forgetOperation();throw new Error(response.error_code);}if(response.kind==="Status"){addEvents(response.events);state.snapshotNumber=response.snapshot_number||null;updateState(response.state);setError(response.error_code?errorMessage(response.error_code):"");document.querySelector("#check").hidden=true;document.querySelector("#confirm").hidden=response.state!=="AwaitingConfirmation";if(["Completed","Failed","NeedsRecovery"].includes(response.state)){clearInterval(state.poll);state.poll=null;}}}catch(error){setError(errorMessage(error));}}
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
function exportVisible(){const data=JSON.stringify({schema:1,operation_id:state.operationId,events:state.events},null,2);const url=URL.createObjectURL(new Blob([data],{type:"application/json"}));const link=document.createElement("a");link.href=url;link.download=`lyra-upgrade-${state.operationId||"diagnostic"}.json`;link.click();URL.revokeObjectURL(url);}
async function recover(action){if(action==="Rollback"&&!confirm(t("rollback_confirm")))return;try{const response=await request("AcknowledgeRecovery",{operation_id:state.operationId,recovery_action:action});if(response.kind==="Rejected")throw new Error(response.error_code);await poll();}catch(error){setError(errorMessage(error));}}
async function restart(){try{await invoke("reboot_system");}catch(error){setError(errorMessage(error));}}
function writeInProgress(){return ["Snapshotting","Applying","ReadyToReboot","ApplyingOffline"].includes(state.currentState);}
window.addEventListener("beforeunload",event=>{if(writeInProgress()){event.preventDefault();event.returnValue="";}});
document.querySelector("#check").addEventListener("click",check);document.querySelector("#confirm").addEventListener("click",confirm);document.querySelector("#restart").addEventListener("click",restart);document.querySelector("#rollback").addEventListener("click",()=>recover("Rollback"));document.querySelector("#keep-current").addEventListener("click",()=>recover("KeepCurrent"));document.querySelector("#details-toggle").addEventListener("click",toggleDetails);document.querySelector("#events-tab").addEventListener("click",()=>chooseTab("events"));document.querySelector("#console-tab").addEventListener("click",()=>chooseTab("console"));document.querySelector(".tabs").addEventListener("keydown",navigateTabs);document.querySelector("#copy").addEventListener("click",copyVisible);document.querySelector("#export").addEventListener("click",exportVisible);document.querySelector("#filter").addEventListener("change",renderDetails);document.querySelector("#search").addEventListener("input",renderDetails);renderPhases("Checking");

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

enableLayoutPreview().then(enabled=>{if(!enabled)resumeOperation();});
