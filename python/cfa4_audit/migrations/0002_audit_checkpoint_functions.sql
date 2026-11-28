-- CFA4 common audit trail, migration 2 (AU-2): checkpoint recording and the erased-personal count.

SET LOCAL ROLE cfa4_audit_owner;

-- Records a checkpoint row after checking it against the chain: the (seq, hash) pair must be an event of the chain
-- (a checkpoint of a different chain, or a typo, is refused). The checkpoint FILE is written by the caller (G2-01
-- section 9); this row is the database-side index of it.
CREATE FUNCTION cfa4_audit.record_checkpoint(p_seq bigint, p_hash bytea, p_file_path text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, cfa4_audit AS $fn$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM cfa4_audit.event e WHERE e.seq = p_seq AND e.hash = p_hash) THEN
        RAISE EXCEPTION 'AUDIT_CHECKPOINT_MISMATCH: seq % with this hash is not in the chain', p_seq
            USING ERRCODE = 'AU004';
    END IF;
    INSERT INTO cfa4_audit.checkpoint (seq, hash, taken_at, file_path)
    VALUES (p_seq, p_hash, clock_timestamp(), p_file_path)
    ON CONFLICT (seq) DO NOTHING;
END
$fn$;

-- How many personal-detail rows are erased: audit.verify reports it (their salted digests can no longer be
-- checked, by design) without the reader role seeing any personal data.
CREATE FUNCTION cfa4_audit.personal_erased_count() RETURNS bigint
LANGUAGE sql SECURITY DEFINER STABLE SET search_path = pg_catalog, cfa4_audit AS $fn$
    SELECT count(*) FROM cfa4_audit.personal_detail WHERE erased_at IS NOT NULL
$fn$;

RESET ROLE;
REVOKE ALL ON FUNCTION cfa4_audit.record_checkpoint(bigint, bytea, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION cfa4_audit.personal_erased_count() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cfa4_audit.record_checkpoint(bigint, bytea, text) TO cfa4_audit_writer;
GRANT EXECUTE ON FUNCTION cfa4_audit.personal_erased_count() TO cfa4_audit_reader, cfa4_audit_writer;
