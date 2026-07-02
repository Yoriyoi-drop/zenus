# Production Review Skill

This skill handles comprehensive production review of Zenus OS. It performs:

## Core Responsibilities
- Assess production readiness (target: 80%+ readiness)
- Review code quality, documentation, and architecture
- Check for production gaps and security issues
- Generate actionable improvement recommendations
- Compare against production standards and best practices

## Review Focus

### Production Readiness Criteria
- **User/Kernel Isolation**: SMAP/SMEP, KPTI, capability systems
- **System Call Completeness**: Complete Linux-compatible syscall set
- **Process Management**: fork, exec, pipe, signal handling, PID namespaces
- **Security Features**: Secure boot, authentication, authorization
- **Storage**: Modern driver models, hotplug support, virtualization
- **Cloud Integration**: Cgroups v2, container runtimes, OCI compliance
- **Dev Experience**: Documentation, CI/CD, testing coverage, versioned APIs
- **Performance**: Hardware acceleration, caching, efficient resource management
- **Monitoring**: Logging, metrics, observability
- **Reliability**: Crash recovery, fault tolerance, high availability

### Security Review
- Kernel hardening (SMAP/SMEP, KPTI)
- Memory safety audit
- Capability-based access control
- Secure boot support
- Driver security review
- Network security features

### Architecture Review
- System layering and modularity
- Resource isolation patterns
- Performance optimization opportunities
- Scalability considerations
- Hardware compatibility

## Output Format
- Production readiness score (0-100)
- Pass/Fail assessment with evidence
- Priority-ranked action items
- Risk assessment
- Milestones and roadmap recommendations

## Integration
Connects with:
- AGENTS.md for project context
- audit.md and audit.txt for historical assessments
- K8s/AWS/GCP production benchmarks (if applicable)
- Industry production standards (CNCF, OWASP, security benchmarks)
