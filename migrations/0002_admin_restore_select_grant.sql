-- 管理端恢复入队（admin_enqueue_restore）在 INSERT 前会以 tasktips_admin
-- 角色检查项目是否已有排队/执行中的恢复任务。0001 只授予了 INSERT，
-- 缺少 SELECT，导致 POST /api/v1/admin/projects/{id}/restores 始终 500。
GRANT SELECT ON TABLE public.restore_jobs TO tasktips_admin;
