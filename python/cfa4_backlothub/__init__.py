"""CFA4 BacklotHub service (CFA4-BACKLOTHUB-001, G2-02 section 2): production management on PostgreSQL.

Every command is one transaction: the business change and its audit event (``cfa4_audit``) commit together or not at all.
"""
