use super::*;

fn invalidate_sql(selector: &str) -> String {
    format!("UPDATE assistant_scope_changes SET revision=revision+1,invalidation_revision=invalidation_revision+1 WHERE singleton=1;
        UPDATE assistant_sources SET state='revoked',revision=revision+1,body=NULL,previous_body=NULL,previous_version=NULL
            WHERE state='active' AND id IN({selector});
        UPDATE sessions SET context_json=NULL,context_revision=context_revision+1,context_run_id=NULL WHERE id IN(
            SELECT r.session_id FROM assistant_research_runs r JOIN assistant_research_inputs i ON i.run_id=r.id WHERE i.source_id IN({selector}));
        UPDATE assistant_research_runs SET state='invalidated',reported_json=NULL,question=NULL,prior_finding=NULL,outcome_reason='source_changed' WHERE id IN(
            SELECT run_id FROM assistant_research_inputs WHERE source_id IN({selector}));
        UPDATE assistant_findings SET state='invalidated',body_json='{{}}' WHERE run_id IN(
            SELECT run_id FROM assistant_research_inputs WHERE source_id IN({selector}));
        UPDATE assistant_research_inputs SET body=NULL,previous_body=NULL WHERE run_id IN(
            SELECT run_id FROM assistant_research_inputs WHERE source_id IN({selector}));
        UPDATE assistant_research_queries SET state='cancelled',reason='source_changed',prior_finding=NULL,question='' WHERE EXISTS(
            SELECT 1 FROM json_each(assistant_research_queries.source_set_json) ref WHERE json_extract(ref.value,'$.sourceId') IN({selector}));")
}

pub(crate) fn migrate_revocation(conn: &Connection) -> Result<()> {
    let source = "SELECT id FROM assistant_sources WHERE session_id=NEW.id";
    conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS assistant_source_session_scope
        AFTER UPDATE OF incognito,kind,archived_at,parent_session_id,is_cron,working_dir,project_id ON sessions
        WHEN EXISTS(SELECT 1 FROM assistant_sources WHERE session_id=NEW.id AND state='active')
        AND (NEW.incognito<>0 OR NEW.kind<>'regular' OR NEW.archived_at IS NOT NULL OR NEW.parent_session_id IS NOT NULL
            OR NEW.is_cron<>0 OR NEW.working_dir IS NOT OLD.working_dir OR NEW.project_id IS NOT OLD.project_id)
        BEGIN {} END;",invalidate_sql(source)))?;
    conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS assistant_source_import_scope AFTER INSERT ON session_import_sources
        WHEN EXISTS(SELECT 1 FROM assistant_sources WHERE session_id=NEW.session_id AND state='active')
        BEGIN {} END;",invalidate_sql("SELECT id FROM assistant_sources WHERE session_id=NEW.session_id")))?;
    conn.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS assistant_source_deleted AFTER DELETE ON assistant_sources
        BEGIN {} END;",
        invalidate_sql("SELECT OLD.id")
    ))?;
    conn.execute_batch("CREATE TRIGGER IF NOT EXISTS assistant_research_context_deleted AFTER DELETE ON sessions
        WHEN OLD.kind='personal_assistant_research'
        BEGIN
            UPDATE assistant_scope_changes SET revision=revision+1,invalidation_revision=invalidation_revision+1 WHERE singleton=1;
            UPDATE assistant_research_runs SET state='invalidated',reported_json=NULL,question=NULL,prior_finding=NULL,outcome_reason='context_deleted' WHERE session_id=OLD.id;
            UPDATE assistant_research_inputs SET body=NULL,previous_body=NULL WHERE run_id IN(SELECT id FROM assistant_research_runs WHERE session_id=OLD.id);
            UPDATE assistant_findings SET state='invalidated',body_json='{}' WHERE run_id IN(SELECT id FROM assistant_research_runs WHERE session_id=OLD.id);
            UPDATE assistant_research_queries SET state='cancelled',reason='context_deleted',question='',prior_finding=NULL WHERE run_id IN(SELECT id FROM assistant_research_runs WHERE session_id=OLD.id);
        END;")?;
    ensure_channel_guards(conn)
}

pub(crate) fn ensure_channel_guards(conn: &Connection) -> Result<()> {
    if !super::super::table_exists(conn, "channel_conversations")?
        || !super::super::table_exists(conn, "assistant_sources")?
    {
        return Ok(());
    }
    for (name, event) in [("insert", "INSERT"), ("update", "UPDATE OF session_id")] {
        let sql =
            invalidate_sql("SELECT id FROM assistant_sources WHERE session_id=NEW.session_id");
        conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS assistant_source_channel_{name} AFTER {event} ON channel_conversations
            WHEN EXISTS(SELECT 1 FROM assistant_sources WHERE session_id=NEW.session_id AND state='active') BEGIN {sql} END;"))?;
    }
    Ok(())
}

impl SessionDB {
    pub fn assistant_scope_change_revision(&self) -> Result<(i64, i64)> {
        Ok(self.read_conn()?.query_row(
            "SELECT revision,invalidation_revision FROM assistant_scope_changes WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
}
