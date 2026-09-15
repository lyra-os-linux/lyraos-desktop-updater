window.LYRA_UPGRADE_CATALOGS = {
  "en-US": {
    "history-truncated": "Older technical lines were removed to keep the history within its storage limit. State changes were preserved.",
    product:"LYRA OS · UPDATE",title:"Keep your system reliable",idle:"Ready",summary_kicker:"SYSTEM MAINTENANCE",checking:"Check for updates",checking_help:"Review the system before making any change.",check:"Check",confirm:"Confirm update",restart:"Restart now",rollback:"Restore snapshot",keep_current:"Keep current system",rollback_confirm:"Restart after restoring the recovery snapshot?",show_details:"Show details",hide_details:"Hide details",details:"Operation details",copy:"Copy",export:"Export",events:"Events",console:"Zypper console",filter_all:"All",filter_warnings:"Warnings",filter_errors:"Errors",search:"Search details",new_lines:"New lines — return to end",planned:"Update ready",planned_help:"Review the plan before authentication and changes.",downloading:"Downloading packages",snapshotting:"Creating recovery snapshot",applying:"Installing updates",awaiting_reboot:"Restart required",verifying:"Verifying restart",blocked:"Update blocked",completed:"System updated",recovered:"System restored",failed:"Update failed",needs_recovery:"Recovery required",no_cancel:"Do not turn off the computer. This step cannot be cancelled.",packages:"packages",space:"Required space",snapshot:"Recovery snapshot",reboot_yes:"restart required",reboot_no:"no restart required",preview_failed:"The update could not be completed. Review the details.",preview_recovery:"The system can be restored from the recovery snapshot.",phase_Checking:"Checking",phase_Preflight:"Safety checks",phase_Downloading:"Download",phase_Snapshotting:"Snapshot",phase_Applying:"Install",phase_AwaitingReboot:"Restart",phase_VerifyingBoot:"Verify",phase_Completed:"Complete"
  },
  "pt-BR": {
    "history-truncated": "Linhas técnicas antigas foram removidas para manter o histórico dentro do limite de armazenamento. As mudanças de estado foram preservadas.",
    product:"LYRA OS · ATUALIZAÇÃO",title:"Mantenha o sistema confiável",idle:"Pronto",summary_kicker:"MANUTENÇÃO DO SISTEMA",checking:"Verificar atualizações",checking_help:"Revise o sistema antes de qualquer alteração.",check:"Verificar",confirm:"Confirmar atualização",restart:"Reiniciar agora",rollback:"Restaurar snapshot",keep_current:"Manter sistema atual",rollback_confirm:"Reiniciar após restaurar o snapshot de recuperação?",show_details:"Mostrar detalhes",hide_details:"Ocultar detalhes",details:"Detalhes da operação",copy:"Copiar",export:"Exportar",events:"Eventos",console:"Console do Zypper",filter_all:"Tudo",filter_warnings:"Avisos",filter_errors:"Erros",search:"Buscar nos detalhes",new_lines:"Novas linhas — voltar ao final",planned:"Atualização pronta",planned_help:"Revise o plano antes da autenticação e das alterações.",downloading:"Baixando pacotes",snapshotting:"Criando snapshot de recuperação",applying:"Instalando atualizações",awaiting_reboot:"Reinicialização necessária",verifying:"Verificando reinicialização",blocked:"Atualização bloqueada",completed:"Sistema atualizado",recovered:"Sistema restaurado",failed:"Falha na atualização",needs_recovery:"Recuperação necessária",no_cancel:"Não desligue o computador. Esta etapa não pode ser cancelada.",packages:"pacotes",space:"Espaço necessário",snapshot:"Snapshot de recuperação",reboot_yes:"reinicialização necessária",reboot_no:"sem reinicialização",preview_failed:"A atualização não pôde ser concluída. Consulte os detalhes.",preview_recovery:"O sistema pode ser restaurado pelo snapshot de recuperação.",phase_Checking:"Verificação",phase_Preflight:"Segurança",phase_Downloading:"Download",phase_Snapshotting:"Snapshot",phase_Applying:"Instalação",phase_AwaitingReboot:"Reinício",phase_VerifyingBoot:"Validação",phase_Completed:"Concluído"
  },
  "es-ES": {
    "history-truncated": "Se eliminaron líneas técnicas antiguas para mantener el historial dentro del límite de almacenamiento. Se conservaron los cambios de estado.",
    product:"LYRA OS · ACTUALIZACIÓN",title:"Mantén el sistema fiable",idle:"Listo",summary_kicker:"MANTENIMIENTO DEL SISTEMA",checking:"Buscar actualizaciones",checking_help:"Revisa el sistema antes de realizar cambios.",check:"Buscar",confirm:"Confirmar actualización",restart:"Reiniciar ahora",rollback:"Restaurar instantánea",keep_current:"Mantener sistema actual",rollback_confirm:"¿Reiniciar después de restaurar la instantánea de recuperación?",show_details:"Mostrar detalles",hide_details:"Ocultar detalles",details:"Detalles de la operación",copy:"Copiar",export:"Exportar",events:"Eventos",console:"Consola de Zypper",filter_all:"Todo",filter_warnings:"Avisos",filter_errors:"Errores",search:"Buscar en los detalles",new_lines:"Nuevas líneas — volver al final",planned:"Actualización lista",planned_help:"Revisa el plan antes de autenticar y cambiar el sistema.",downloading:"Descargando paquetes",snapshotting:"Creando instantánea de recuperación",applying:"Instalando actualizaciones",awaiting_reboot:"Es necesario reiniciar",verifying:"Verificando el reinicio",blocked:"Actualización bloqueada",completed:"Sistema actualizado",recovered:"Sistema restaurado",failed:"Falló la actualización",needs_recovery:"Se requiere recuperación",no_cancel:"No apagues el equipo. Este paso no se puede cancelar.",packages:"paquetes",space:"Espacio necesario",snapshot:"Instantánea de recuperación",reboot_yes:"reinicio necesario",reboot_no:"sin reinicio",preview_failed:"No se pudo completar la actualización. Revisa los detalles.",preview_recovery:"El sistema puede restaurarse desde la instantánea de recuperación.",phase_Checking:"Comprobación",phase_Preflight:"Seguridad",phase_Downloading:"Descarga",phase_Snapshotting:"Instantánea",phase_Applying:"Instalación",phase_AwaitingReboot:"Reinicio",phase_VerifyingBoot:"Verificación",phase_Completed:"Completado"
  }
};

// Version upgrade review and read-only discovery.
for (const [locale, additions] of Object.entries({
  "en-US": {
    "check_release": "Check for a new version",
    "plan_release": "Review version upgrade",
    "release_checking": "Checking the signed release offer…",
    "release_available": "New version available",
    "snapshot_scope": "The system snapshot is not a backup of your personal files. Verify an external backup before upgrading to another version.",
    "backup_ack": "I understand the snapshot limits and have reviewed my backup.",
    "review_packages": "Review package changes",
    "review_repositories": "Review repositories",
    "third_party_disabled": "Only repositories authorized by the signed release are used. Other repositories are preserved in the recovery copy and remain disabled after the upgrade.",
    "no_package_changes": "No package changes.",
    "action_Install": "Install",
    "action_Upgrade": "Update",
    "action_Remove": "Remove",
    "action_Downgrade": "Downgrade",
    "action_Reinstall": "Reinstall",
    "error_MANIFEST_SEQUENCE_INVALID": "The release trust record could not be validated. The upgrade is blocked.",
    "error_MANIFEST_CHANGED": "The offered release changed. Check again and review the new plan.",
    "error_QUERY_UNAVAILABLE": "The status service is unavailable. Try again after checking the Lyra Upgrade installation.",
    "error_BACKUP_ACK_REQUIRED": "Review the backup notice before confirming.",
    "error_RELEASE_NOT_AVAILABLE": "No supported version upgrade is currently offered."
  },
  "pt-BR": {
    "check_release": "Buscar nova versão",
    "plan_release": "Revisar mudança de versão",
    "release_checking": "Consultando a oferta de versão assinada…",
    "release_available": "Nova versão disponível",
    "snapshot_scope": "O snapshot do sistema não é um backup dos seus arquivos pessoais. Verifique um backup externo antes de mudar de versão.",
    "backup_ack": "Entendo os limites do snapshot e revisei meu backup.",
    "review_packages": "Revisar alterações nos pacotes",
    "review_repositories": "Revisar repositórios",
    "third_party_disabled": "Somente os repositórios autorizados pela versão assinada serão usados. Os demais ficam preservados na cópia de recuperação e desabilitados após a mudança de versão.",
    "no_package_changes": "Nenhuma alteração nos pacotes.",
    "action_Install": "Instalar",
    "action_Upgrade": "Atualizar",
    "action_Remove": "Remover",
    "action_Downgrade": "Rebaixar",
    "action_Reinstall": "Reinstalar",
    "error_MANIFEST_SEQUENCE_INVALID": "Não foi possível validar o registro de confiança das versões. A mudança de versão foi bloqueada.",
    "error_MANIFEST_CHANGED": "A versão oferecida mudou. Consulte novamente e revise o novo plano.",
    "error_QUERY_UNAVAILABLE": "O serviço de consulta está indisponível. Confira a instalação do Lyra Upgrade e tente novamente.",
    "error_BACKUP_ACK_REQUIRED": "Revise o aviso de backup antes de confirmar.",
    "error_RELEASE_NOT_AVAILABLE": "Nenhuma mudança de versão compatível está disponível no momento."
  },
  "es-ES": {
    "check_release": "Buscar nueva versión",
    "plan_release": "Revisar cambio de versión",
    "release_checking": "Consultando la oferta de versión firmada…",
    "release_available": "Nueva versión disponible",
    "snapshot_scope": "La instantánea del sistema no es una copia de seguridad de tus archivos personales. Verifica una copia externa antes de cambiar de versión.",
    "backup_ack": "Entiendo los límites de la instantánea y he revisado mi copia de seguridad.",
    "review_packages": "Revisar cambios de paquetes",
    "review_repositories": "Revisar repositorios",
    "third_party_disabled": "Solo se usarán los repositorios autorizados por la versión firmada. Los demás se conservan en la copia de recuperación y quedan deshabilitados tras el cambio de versión.",
    "no_package_changes": "Sin cambios de paquetes.",
    "action_Install": "Instalar",
    "action_Upgrade": "Actualizar",
    "action_Remove": "Eliminar",
    "action_Downgrade": "Degradar",
    "action_Reinstall": "Reinstalar",
    "error_MANIFEST_SEQUENCE_INVALID": "No se pudo validar el registro de confianza de versiones. El cambio de versión está bloqueado.",
    "error_MANIFEST_CHANGED": "La versión ofrecida cambió. Consulta de nuevo y revisa el nuevo plan.",
    "error_QUERY_UNAVAILABLE": "El servicio de consulta no está disponible. Revisa la instalación de Lyra Upgrade e inténtalo de nuevo.",
    "error_BACKUP_ACK_REQUIRED": "Revisa el aviso de copia de seguridad antes de confirmar.",
    "error_RELEASE_NOT_AVAILABLE": "No hay un cambio de versión compatible disponible."
  }
})) Object.assign(window.LYRA_UPGRADE_CATALOGS[locale], additions);

Object.assign(window.LYRA_UPGRADE_CATALOGS["en-US"], {release_cached:"Cached offer; reconnect before planning",configuration_artifacts:"RPM configuration files to review"});
Object.assign(window.LYRA_UPGRADE_CATALOGS["pt-BR"], {release_cached:"Oferta em cache; conecte-se antes de planejar",configuration_artifacts:"Arquivos de configuração RPM para revisar"});
Object.assign(window.LYRA_UPGRADE_CATALOGS["es-ES"], {release_cached:"Oferta en caché; conecta antes de planificar",configuration_artifacts:"Archivos de configuración RPM para revisar"});
