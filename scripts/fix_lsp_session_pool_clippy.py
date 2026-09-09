from pathlib import Path

path = Path("src-tauri/src/coding_lsp.rs")
text = path.read_text(encoding="utf-8")
old = '''    let mut duplicate = None;
    let mut rejected = false;
    {
        let mut pool = lsp_session_pool().lock().await;
        if let Some(entry) = pool.entries.get_mut(&key) {
            entry.last_used = Instant::now();
            duplicate = Some(SessionLease {
                key: key.clone(),
                session: Arc::clone(&entry.session),
            });
        } else if pool.entries.len() < MAX_POOLED_LSP_SESSIONS {
'''
new = '''    let mut duplicate_session = None;
    let mut rejected = false;
    {
        let mut pool = lsp_session_pool().lock().await;
        if let Some(entry) = pool.entries.get_mut(&key) {
            entry.last_used = Instant::now();
            duplicate_session = Some(Arc::clone(&entry.session));
        } else if pool.entries.len() < MAX_POOLED_LSP_SESSIONS {
'''
if text.count(old) != 1:
    raise RuntimeError("expected exactly one duplicate-session acquisition block")
text = text.replace(old, new, 1)
old = '''    if let Some(duplicate) = duplicate {
        close_session_handles(vec![started]).await;
        return Ok(duplicate);
    }
'''
new = '''    if let Some(session) = duplicate_session {
        close_session_handles(vec![started]).await;
        return Ok(SessionLease { key, session });
    }
'''
if text.count(old) != 1:
    raise RuntimeError("expected exactly one duplicate-session return block")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
