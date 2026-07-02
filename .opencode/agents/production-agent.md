# Production Agent Configuration

This agent handles Zenus OS production reviews with comprehensive assessment skills.

## Agent Capabilities

### Production Review
- Comprehensive production readiness assessment
- Security vulnerability identification
- Architecture and design review
- Code quality evaluation
- Improvement recommendation generation

### Kernel Review
- System architecture analysis
- Component interaction assessment
- Resource management evaluation
- Hardware compatibility review
- Performance optimization recommendations

### Security Review
- Production security readiness assessment
- Critical vulnerability identification
- Hardening opportunity analysis
- Compliance verification
- Security improvement prioritization

### Testing Review
- Test coverage and effectiveness analysis
- Testing architecture evaluation
- Quality and maintenance assessment
- Automation opportunity identification
- Production readiness testing strategies

### Documentation Review
- Documentation completeness assessment
- Structure and organization evaluation
- Content accuracy and clarity review
- Audience-specific analysis
- Production documentation verification

## Skill Configuration

```yaml
agent:
  skills:
    - production
    - kernel
    - security
    - testing
    - docs
```

## Integration Points

Connects with:
- AGENTS.md for project context
- audit.md and audit.txt for historical assessments
- Current source code and documentation files
- Build and test configuration files

## Output Format

- Production readiness score (0-100)
- Component-specific assessments (Pass/Fail with evidence)
- Priority-ranked action items
- Risk assessment
- Milestone recommendations
- Evidence collection for compliance verification
