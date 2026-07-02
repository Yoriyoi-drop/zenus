# Security Review Skill

This skill conducts comprehensive security assessments of Zenus OS, focusing on production security readiness and vulnerability assessment.

## Core Responsibilities
- Identify security gaps and vulnerabilities
- Assess production security readiness
- Review access control and isolation mechanisms
- Evaluate hardening opportunities
- Generate security improvement recommendations

## Review Focus

### Critical Security Issues
#### User/Kernel Isolation
- SMAP/SMEP support
- KPTI (Kernel Page Table Isolation)
- Ring 0/ring 3 protection boundaries
- Address space layout randomization

#### Memory Safety
- Unsafe block audit and mitigation
- Bounds checking and validation
- Buffer overflow protection
- Use-after-free prevention

#### Access Control
- Capability system implementation
- Mandatory/Discretionary access control
- Privilege escalation prevention
- Security context management

### Production Hardening
- Secure boot support
- Secure firmware updates
- Runtime integrity verification
- Auditing and logging
- Network security (iptables, firewall, IPsec)

### Driver Security
- Driver sandboxing
- Privilege separation
- Input validation
- Privilege escalation prevention

## Output Format
- Security readiness score (0-100)
- Critical vulnerability assessment
- Hardening recommendations
- Compliance checks (CIS benchmarks, OWASP)
- Incident response procedures

## Integration
Connects with:
- Production review skill for overall assessment
- AGENTS.md for security policies
- audit.md and audit.txt for historical security findings
- OS security benchmarks (Linux security baseline, OS hardening guides)
