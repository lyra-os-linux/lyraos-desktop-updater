(() => {
  const messages = {
    "en-US": {
      error_EVENT_LOG_READ_FAILED: "Some operation details could not be read. The available history is shown; the update state is unchanged.",
      error_EVENT_LOG_WRITE_FAILED: "Some operation details could not be saved. Keep this window open to retain the available details.",
      error_SNAPSHOT_RECOVERY_UNSUPPORTED: "This snapshot predates automatic recovery verification. Use administrative recovery.",
      error_ROLLBACK_INTENT_INCOMPLETE: "Rollback preparation was interrupted. Review the boot selection before trying recovery again.",
      error_ROLLBACK_RESULT_INVALID: "Rollback preparation was interrupted. Review the boot selection before trying recovery again.",
      error_POST_BOOT_ROLLBACK_IDENTITY_FAILED: "The restored system does not match the expected snapshot or version.",
      error_POST_BOOT_IDENTITY_FAILED: "The restored system does not match the expected snapshot or version.",
      error_SNAPSHOT_IDENTITY_INVALID: "The restored system does not match the expected snapshot or version.",
      error_SNAPSHOT_IDENTITY_MISMATCH: "The restored system does not match the expected snapshot or version.",
      error_RECOVERY_STATE_NOT_PERSISTENT: "Recovery records must be stored outside the root snapshot.",
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
      error_EVENT_LOG_READ_FAILED: "Não foi possível ler parte dos detalhes da operação. O histórico disponível é exibido; o estado da atualização não mudou.",
      error_EVENT_LOG_WRITE_FAILED: "Não foi possível salvar parte dos detalhes da operação. Mantenha esta janela aberta para preservar os detalhes disponíveis.",
      error_SNAPSHOT_RECOVERY_UNSUPPORTED: "Este snapshot é anterior à verificação automática de recuperação. Use a recuperação administrativa.",
      error_ROLLBACK_INTENT_INCOMPLETE: "A preparação do rollback foi interrompida. Revise a seleção de boot antes de tentar recuperar novamente.",
      error_ROLLBACK_RESULT_INVALID: "A preparação do rollback foi interrompida. Revise a seleção de boot antes de tentar recuperar novamente.",
      error_POST_BOOT_ROLLBACK_IDENTITY_FAILED: "O sistema restaurado não corresponde ao snapshot ou à versão esperada.",
      error_POST_BOOT_IDENTITY_FAILED: "O sistema restaurado não corresponde ao snapshot ou à versão esperada.",
      error_SNAPSHOT_IDENTITY_INVALID: "O sistema restaurado não corresponde ao snapshot ou à versão esperada.",
      error_SNAPSHOT_IDENTITY_MISMATCH: "O sistema restaurado não corresponde ao snapshot ou à versão esperada.",
      error_RECOVERY_STATE_NOT_PERSISTENT: "Os registros de recuperação precisam ficar fora do snapshot da raiz.",
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
      error_EVENT_LOG_READ_FAILED: "No se pudo leer parte de los detalles de la operación. Se muestra el historial disponible; el estado de la actualización no cambió.",
      error_EVENT_LOG_WRITE_FAILED: "No se pudo guardar parte de los detalles de la operación. Mantén esta ventana abierta para conservar los detalles disponibles.",
      error_SNAPSHOT_RECOVERY_UNSUPPORTED: "Esta instantánea es anterior a la verificación automática de recuperación. Usa la recuperación administrativa.",
      error_ROLLBACK_INTENT_INCOMPLETE: "Se interrumpió la preparación de la restauración. Revisa la selección de arranque antes de intentar recuperar de nuevo.",
      error_ROLLBACK_RESULT_INVALID: "Se interrumpió la preparación de la restauración. Revisa la selección de arranque antes de intentar recuperar de nuevo.",
      error_POST_BOOT_ROLLBACK_IDENTITY_FAILED: "El sistema restaurado no coincide con la instantánea o la versión esperada.",
      error_POST_BOOT_IDENTITY_FAILED: "El sistema restaurado no coincide con la instantánea o la versión esperada.",
      error_SNAPSHOT_IDENTITY_INVALID: "El sistema restaurado no coincide con la instantánea o la versión esperada.",
      error_SNAPSHOT_IDENTITY_MISMATCH: "El sistema restaurado no coincide con la instantánea o la versión esperada.",
      error_RECOVERY_STATE_NOT_PERSISTENT: "Los registros de recuperación deben estar fuera de la instantánea raíz.",
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

Object.assign(window.LYRA_UPGRADE_CATALOGS["en-US"], {"error_OPERATION_INTERRUPTED": "The previous operation was interrupted. Review recovery before continuing.", "error_PACKAGE_PRESERVATION_FAILED": "The installed packages differ from the reviewed plan. Recovery is required.", "error_POST_BOOT_INVENTORY_FAILED": "Package preservation could not be verified after restart. Review recovery."});

Object.assign(window.LYRA_UPGRADE_CATALOGS["pt-BR"], {"error_OPERATION_INTERRUPTED": "A operação anterior foi interrompida. Revise a recuperação antes de continuar.", "error_PACKAGE_PRESERVATION_FAILED": "Os pacotes instalados diferem do plano revisado. É necessário revisar a recuperação.", "error_POST_BOOT_INVENTORY_FAILED": "Não foi possível confirmar a preservação dos pacotes após reiniciar. Revise a recuperação."});

Object.assign(window.LYRA_UPGRADE_CATALOGS["es-ES"], {"error_OPERATION_INTERRUPTED": "La operación anterior se interrumpió. Revisa la recuperación antes de continuar.", "error_PACKAGE_PRESERVATION_FAILED": "Los paquetes instalados difieren del plan revisado. Se requiere recuperación.", "error_POST_BOOT_INVENTORY_FAILED": "No se pudo verificar la conservación de paquetes tras reiniciar. Revisa la recuperación."});

Object.assign(window.LYRA_UPGRADE_CATALOGS["en-US"], {"error_MANIFEST_INVALID": "The release offer could not be authenticated. Check your connection and try again.", "error_STORAGE_LAYOUT_UNKNOWN": "The storage layout could not be verified.", "error_ROOT_NOT_WRITABLE": "The system volume must be writable and healthy.", "error_HOME_NOT_ISOLATED": "Personal files must be on a volume or subvolume separate from the system snapshot.", "error_INSUFFICIENT_BOOT_SPACE": "Free at least 256 MiB in /boot.", "error_INSUFFICIENT_ESP_SPACE": "Free at least 32 MiB in the EFI partition.", "error_INSUFFICIENT_SPACE": "There is not enough space for this update and its recovery snapshot.", "error_BATTERY_TOO_LOW": "Connect the power supply before updating.", "error_BATTERY_STATE_UNKNOWN": "The battery level is unknown. Connect the power supply.", "error_ROOT_NOT_BTRFS": "System recovery requires Btrfs.", "error_SNAPPER_UNAVAILABLE": "The system recovery snapshot configuration is unavailable.", "error_RPM_DATABASE_UNHEALTHY": "The package database requires diagnosis before updating.", "error_PACKAGE_MANAGER_BUSY": "Another package operation is running. Try again when it finishes.", "error_SECURE_BOOT_STATE_UNKNOWN": "The Secure Boot state could not be verified.", "error_UNAUTHORIZED_REMOVAL": "The plan would remove a package without release authorization.", "error_UNAUTHORIZED_VENDOR_CHANGE": "The plan would change a package vendor without authorization.", "error_UNAUTHORIZED_DOWNGRADE": "The plan would install an older package version.", "error_LOCKSTEP_VIOLATION": "Required packages must be updated together.", "error_REPOSITORY_METADATA_INVALID": "Repository metadata could not be validated.", "error_REPOSITORY_KEY_UNTRUSTED": "The repository signing key is not trusted.", "error_UNSUPPORTED_EDITION": "This updater supports Lyra OS Desktop.", "error_UNSUPPORTED_ARCHITECTURE": "This updater supports x86_64 systems.", "error_SOLVER_FAILED": "The package solver could not produce a safe update plan.", "error_UNSUPPORTED_SOLVER_SCHEMA": "The package solver format is not supported."});

Object.assign(window.LYRA_UPGRADE_CATALOGS["pt-BR"], {"error_MANIFEST_INVALID": "Não foi possível autenticar a oferta de versão. Confira a conexão e tente novamente.", "error_STORAGE_LAYOUT_UNKNOWN": "Não foi possível verificar a disposição dos volumes.", "error_ROOT_NOT_WRITABLE": "O volume do sistema precisa estar gravável e sem degradação.", "error_HOME_NOT_ISOLATED": "Os arquivos pessoais precisam estar em volume ou subvolume separado do snapshot do sistema.", "error_INSUFFICIENT_BOOT_SPACE": "Libere pelo menos 256 MiB em /boot.", "error_INSUFFICIENT_ESP_SPACE": "Libere pelo menos 32 MiB na partição EFI.", "error_INSUFFICIENT_SPACE": "Não há espaço suficiente para a atualização e seu snapshot de recuperação.", "error_BATTERY_TOO_LOW": "Conecte a fonte de energia antes de atualizar.", "error_BATTERY_STATE_UNKNOWN": "O nível da bateria é desconhecido. Conecte a fonte de energia.", "error_ROOT_NOT_BTRFS": "A recuperação do sistema exige Btrfs.", "error_SNAPPER_UNAVAILABLE": "A configuração dos snapshots de recuperação está indisponível.", "error_RPM_DATABASE_UNHEALTHY": "O banco de pacotes precisa de diagnóstico antes da atualização.", "error_PACKAGE_MANAGER_BUSY": "Outra operação de pacotes está em andamento. Tente novamente quando terminar.", "error_SECURE_BOOT_STATE_UNKNOWN": "Não foi possível verificar o estado do Secure Boot.", "error_UNAUTHORIZED_REMOVAL": "O plano removeria um pacote sem autorização da versão.", "error_UNAUTHORIZED_VENDOR_CHANGE": "O plano mudaria o fornecedor de um pacote sem autorização.", "error_UNAUTHORIZED_DOWNGRADE": "O plano instalaria uma versão mais antiga de um pacote.", "error_LOCKSTEP_VIOLATION": "Os pacotes dependentes precisam ser atualizados em conjunto.", "error_REPOSITORY_METADATA_INVALID": "Não foi possível validar os metadados do repositório.", "error_REPOSITORY_KEY_UNTRUSTED": "A chave de assinatura do repositório não é confiável.", "error_UNSUPPORTED_EDITION": "Este atualizador oferece suporte ao Lyra OS Desktop.", "error_UNSUPPORTED_ARCHITECTURE": "Este atualizador oferece suporte a sistemas x86_64.", "error_SOLVER_FAILED": "O resolvedor de pacotes não produziu um plano de atualização seguro.", "error_UNSUPPORTED_SOLVER_SCHEMA": "O formato do resolvedor de pacotes não é suportado."});

Object.assign(window.LYRA_UPGRADE_CATALOGS["es-ES"], {"error_MANIFEST_INVALID": "No se pudo autenticar la oferta de versión. Comprueba la conexión e inténtalo de nuevo.", "error_STORAGE_LAYOUT_UNKNOWN": "No se pudo verificar la distribución de los volúmenes.", "error_ROOT_NOT_WRITABLE": "El volumen del sistema debe permitir escritura y no estar degradado.", "error_HOME_NOT_ISOLATED": "Los archivos personales deben estar en un volumen o subvolumen separado de la instantánea del sistema.", "error_INSUFFICIENT_BOOT_SPACE": "Libera al menos 256 MiB en /boot.", "error_INSUFFICIENT_ESP_SPACE": "Libera al menos 32 MiB en la partición EFI.", "error_INSUFFICIENT_SPACE": "No hay espacio suficiente para la actualización y su instantánea de recuperación.", "error_BATTERY_TOO_LOW": "Conecta la alimentación antes de actualizar.", "error_BATTERY_STATE_UNKNOWN": "Se desconoce el nivel de batería. Conecta la alimentación.", "error_ROOT_NOT_BTRFS": "La recuperación del sistema requiere Btrfs.", "error_SNAPPER_UNAVAILABLE": "La configuración de instantáneas de recuperación no está disponible.", "error_RPM_DATABASE_UNHEALTHY": "La base de paquetes necesita diagnóstico antes de actualizar.", "error_PACKAGE_MANAGER_BUSY": "Hay otra operación de paquetes en curso. Inténtalo cuando termine.", "error_SECURE_BOOT_STATE_UNKNOWN": "No se pudo verificar el estado de Secure Boot.", "error_UNAUTHORIZED_REMOVAL": "El plan eliminaría un paquete sin autorización de la versión.", "error_UNAUTHORIZED_VENDOR_CHANGE": "El plan cambiaría el proveedor de un paquete sin autorización.", "error_UNAUTHORIZED_DOWNGRADE": "El plan instalaría una versión anterior de un paquete.", "error_LOCKSTEP_VIOLATION": "Los paquetes dependientes deben actualizarse juntos.", "error_REPOSITORY_METADATA_INVALID": "No se pudieron validar los metadatos del repositorio.", "error_REPOSITORY_KEY_UNTRUSTED": "La clave de firma del repositorio no es de confianza.", "error_UNSUPPORTED_EDITION": "Este actualizador admite Lyra OS Desktop.", "error_UNSUPPORTED_ARCHITECTURE": "Este actualizador admite sistemas x86_64.", "error_SOLVER_FAILED": "El solucionador de paquetes no pudo generar un plan seguro.", "error_UNSUPPORTED_SOLVER_SCHEMA": "No se admite el formato del solucionador de paquetes."});
