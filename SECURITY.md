# Security Policy

## Reporting a Security Vulnerability

Please do not disclose security vulnerabilities through public GitHub issues.

Report security issues privately to the repository maintainers.

When reporting a vulnerability, include:

- A description of the issue
- Steps to reproduce it
- Potential impact
- Any relevant logs or proof of concept
- Suggested mitigation, if known

## Secrets

Never commit:

- API keys
- Access tokens
- Passwords
- Private keys
- `.env` files
- Cloud credentials
- Database credentials

If a secret is accidentally committed, assume it is compromised and rotate it immediately.
