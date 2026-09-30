-- 异常登录/访问审计：复用 audit_events 记录失败事件，无需改表结构
-- （actor_user_id 已可为 NULL，tasktips_auth 可经 audit_events_internal 写入）。
-- 本迁移仅补齐服务端过滤与保留所需的索引；失败事件的写入、查询、清理
-- 见 persistence（record_auth_failure / prune_old_auth_failures）与 worker。
--
-- 新增动作（action）：
--   auth.login_failed   （source: login | admin_login | web_login）
--   auth.refresh_failed （source: refresh | admin_refresh | web_refresh，含 token 重放）
--   auth.reauth_failed  （source: admin_reauth）
-- 元数据仅存操作性字段（camelCase）：reason / source / emailHash / deviceId / clientHash，
-- 永不存密码、令牌明文、邮箱明文、IP 明文。

CREATE INDEX IF NOT EXISTS audit_events_action_created_idx
    ON public.audit_events (action, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS audit_events_created_idx
    ON public.audit_events (created_at DESC, id DESC);

-- 失败审计保留清理仅允许 worker 删除三类失败动作，避免误删成功审计。
-- tasktips_worker 在 0001 中只有 INSERT/UPDATE，此处补齐受限 DELETE。
GRANT DELETE ON TABLE public.audit_events TO tasktips_worker;

DROP POLICY IF EXISTS audit_events_worker_delete ON public.audit_events;
CREATE POLICY audit_events_worker_delete ON public.audit_events FOR DELETE
    USING (
        CURRENT_USER = 'tasktips_worker'::name
        AND action IN ('auth.login_failed', 'auth.refresh_failed', 'auth.reauth_failed')
    );
