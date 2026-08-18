(() => {
  const messages = {
    "en-US": {
      error_AUTHORIZATION: "Authorization was cancelled or denied.",
      error_CONFIRMATION_REQUIRED: "Confirm the update before continuing.",
      error_DISCOVERY_FAILED: "Lyra could not inspect this system safely.",
      error_EXECUTION_FAILED: "The update could not be completed. Your recovery snapshot was preserved.",
      error_INVALID_STATE: "The operation changed state. Reopen Lyra Upgrade to resume it.",
      error_OPERATION_NOT_FOUND: "This operation is no longer available to this user.",
      error_PLAN_HASH_MISMATCH: "The update plan changed and must be checked again.",
      error_PLAN_NOT_AVAILABLE: "The saved update plan is no longer available.",
      error_PREFLIGHT_BLOCKED: "The safety checks blocked this update. Open the details for more information.",
      error_STATE_READ_FAILED: "The saved operation state could not be read safely.",
      error_STATE_WRITE_FAILED: "The operation could not be saved safely.",
      error_UNKNOWN: "Lyra Upgrade could not communicate with the update service."
    },
    "pt-BR": {
      error_AUTHORIZATION: "A autorização foi cancelada ou negada.",
      error_CONFIRMATION_REQUIRED: "Confirme a atualização antes de continuar.",
      error_DISCOVERY_FAILED: "O Lyra não conseguiu inspecionar este sistema com segurança.",
      error_EXECUTION_FAILED: "A atualização não pôde ser concluída. O snapshot de recuperação foi preservado.",
      error_INVALID_STATE: "A operação mudou de estado. Reabra o Lyra Upgrade para retomá-la.",
      error_OPERATION_NOT_FOUND: "Esta operação não está mais disponível para este usuário.",
      error_PLAN_HASH_MISMATCH: "O plano de atualização mudou e precisa ser verificado novamente.",
      error_PLAN_NOT_AVAILABLE: "O plano de atualização salvo não está mais disponível.",
      error_PREFLIGHT_BLOCKED: "As verificações de segurança bloquearam esta atualização. Abra os detalhes para saber mais.",
      error_STATE_READ_FAILED: "Não foi possível ler com segurança o estado salvo da operação.",
      error_STATE_WRITE_FAILED: "Não foi possível salvar a operação com segurança.",
      error_UNKNOWN: "O Lyra Upgrade não conseguiu se comunicar com o serviço de atualização."
    },
    "es-ES": {
      error_AUTHORIZATION: "La autorización fue cancelada o denegada.",
      error_CONFIRMATION_REQUIRED: "Confirma la actualización antes de continuar.",
      error_DISCOVERY_FAILED: "Lyra no pudo inspeccionar este sistema de forma segura.",
      error_EXECUTION_FAILED: "No se pudo completar la actualización. Se conservó la instantánea de recuperación.",
      error_INVALID_STATE: "La operación cambió de estado. Vuelve a abrir Lyra Upgrade para reanudarla.",
      error_OPERATION_NOT_FOUND: "Esta operación ya no está disponible para este usuario.",
      error_PLAN_HASH_MISMATCH: "El plan de actualización cambió y debe comprobarse de nuevo.",
      error_PLAN_NOT_AVAILABLE: "El plan de actualización guardado ya no está disponible.",
      error_PREFLIGHT_BLOCKED: "Las comprobaciones de seguridad bloquearon esta actualización. Abre los detalles para más información.",
      error_STATE_READ_FAILED: "No se pudo leer de forma segura el estado guardado de la operación.",
      error_STATE_WRITE_FAILED: "No se pudo guardar la operación de forma segura.",
      error_UNKNOWN: "Lyra Upgrade no pudo comunicarse con el servicio de actualización."
    }
  };
  const generated = {
    "en-US": {
      retry: "The safety check could not be completed. Resolve the condition and check again.",
      recovery: "The update stopped after the recovery snapshot. Review details and choose whether to restore it.",
      restart: "Zypper was updated and must be restarted before the operation can continue."
    },
    "pt-BR": {
      retry: "A verificação de segurança não pôde ser concluída. Corrija a condição e verifique novamente.",
      recovery: "A atualização parou após o snapshot de recuperação. Revise os detalhes e escolha se deseja restaurá-lo.",
      restart: "O Zypper foi atualizado e precisa ser reiniciado antes de continuar a operação."
    },
    "es-ES": {
      retry: "No se pudo completar la comprobación de seguridad. Corrige la condición y vuelve a comprobar.",
      recovery: "La actualización se detuvo después de la instantánea de recuperación. Revisa los detalles y elige si deseas restaurarla.",
      restart: "Zypper se actualizó y debe reiniciarse antes de continuar la operación."
    }
  };
  const retryCodes=["PLAN_CHANGED","METADATA_REFRESH_FAILED","PLAN_REVALIDATION_FAILED","DOWNLOAD_FAILED","TRANSACTION_BUSY","SYSTEM_UPDATE_BUSY"];
  const recoveryCodes=["SNAPSHOT_FAILED","ZYPPER_APPLY_FAILED","INITRAMFS_FAILED","BOOTLOADER_FAILED","INVALID_STATE_TRANSITION","OFFLINE_STAGE_FAILED"];
  for (const [locale, additions] of Object.entries(messages)) {
    for(const code of retryCodes)additions[`error_${code}`]=generated[locale].retry;
    for(const code of recoveryCodes)additions[`error_${code}`]=generated[locale].recovery;
    additions.error_ZYPPER_RESTART_REQUIRED=generated[locale].restart;
    additions.error_REBOOT_FAILED=additions.error_UNKNOWN;
    additions.error_ROLLBACK_FAILED=generated[locale].recovery;
    additions.error_RECOVERY_DECLINED=generated[locale].recovery;
    additions.error_CANCELLED=generated[locale].retry;
    Object.assign(window.LYRA_UPGRADE_CATALOGS[locale], additions);
  }
})();
