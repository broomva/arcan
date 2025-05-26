---
description: 
globs: 
alwaysApply: true
---
🧠 1. Evidence-Driven Craftsmanship

(Rationalism & Empiricism → Test, Measure, Learn)
	•	Observability First
	•	Instrument every service or UI flow with structured logs, metrics (Prometheus, OpenTelemetry), and distributed traces so hypotheses can be validated or refuted.
	•	Test-Driven Mindset
	•	Write TDD-style unit tests (pytest/​pytest-asyncio) and integration tests before feature code.
	•	Gate merges on ≥90% coverage and passing performance budgets.
	•	Data-Backed Decisions
	•	Use A/B testing for UX changes (e.g. onboarding tweak) and track via analytics dashboards.
	•	Avoid “cargo-cult” patterns—adopt libraries only once they’ve proved impact in your context.

⸻

🌐 2. Systemic & Holistic Architecture

(Systems Thinking & Holism → Design for the Whole)
	•	Modular “Spellbook” Services
	•	Break features into small, versioned modules (à la Arcan spells).
	•	Expose clear interfaces (JSON-schema/​Pydantic) and isolate via containers or cgroups.
	•	Event-Driven Integrations
	•	Favor message buses (Kafka, RabbitMQ) or change-data-capture streams so services remain loosely coupled and emergent behaviors surface in staging.
	•	Dependency Injection & Patterns
	•	Factory/Strategy for interchangeable implementations (e.g. different payment rails).
	•	Use DI (FastAPI Depends, Inversify in TS) so components can be swapped for testing.

⸻

🚀 3. Pragmatic Iteration

(Pragmatism → Ship, Learn, Evolve)
	•	Prototype-First UX
	•	Sketch in Figma/Miro; validate with 3–5 target users before writing a line of code.
	•	MVP Releases
	•	Release minimal viable versions of features (e.g. basic WedIA chat) and expand based on real usage patterns.
	•	Continuous Feedback Loops
	•	Embed in-app feedback prompts; schedule weekly “demo & decide” syncs to pivot quickly.

⸻

❌ 4. Falsifiable Features

(Popperian Falsification → Assume Wrong Until Proven Right)
	•	Retrospectives & Postmortems
	•	Hold blameless reviews after incidents, catalog root causes, update guardrails (retry policies, health checks).
	•	Automated Rollbacks
	•	CI pipelines must include canary gates: if error rate >1% in canary ⇒ auto-rollback.
	•	Decision Logs
	•	Record major design choices in docs/DECISIONS.md with the date, context, and expected outcome.

⸻

🎨 5. Empathetic Interfaces

(Phenomenology → Center the User’s Experience)
	•	Accessibility by Design
	•	Run axe/Lighthouse audits in CI; target WCAG AA.
	•	Semantic HTML, ARIA roles, keyboard navigation.
	•	Localization & Tone
	•	Spanish/Bogotán accent copy; real-time translation flows in Theo inspired your bilingual design.
	•	Keep microcopy clear and human-centered—no jargon.
	•	User Research Integration
	•	Embed short usability tests in each sprint; recruit from real Wedi or Theo users.

⸻

🔄 6. Reflective Practice

(Schön’s Reflective Practitioner → Learn from Each Iteration)
	•	Knowledge Spikes
	•	Reserve 10% of sprint capacity for tech research (new Pinecone features, Spark optimizations).
	•	Decision Annotations
	•	In-code “why” comments linking back to decision log entries.
	•	Pair-Programming & Rotation
	•	Rotate pairing partners monthly to spread domain knowledge.

⸻

🛡 7. Resilient & Stoic Engineering

(Stoicism → Focus on Controllables, Graceful Degradation)
	•	Robust Error Handling
	•	Central error-class hierarchy; recover with retries, backoffs, or fallback UIs.
	•	Prioritized Backlog
	•	Use ICE scoring (Impact, Confidence, Effort) to decide what not to build this cycle.
	•	Graceful Degradation
	•	If a microservice is down, the UI still shows cached balances and an “offline mode” banner.

⸻

📋 8. Concrete Tech & Code Standards

Domain	Standard	Rationale
Project Structure	Feature-based directories (features/payments, lib/ml)	Easier holistic traces (systems thinking)
Versioning & Branching	Trunk-based with short feature branches + semantic tags (v2.3.0)	Pragmatic, low-overhead releases
Python Style	Black + isort + flake8 in pre-commit; type hints (≥3.10) everywhere	Ensures consistency and evidence-driven refactors
FastAPI Patterns	Pydantic models in models/, routers in api/, services in services/	Clear separation of concerns
Async Best Practices	async/await for I/O, limit concurrency with semaphores	Controllable resource use (Stoicism)
Next.js & TS	App Router, Server/Client components, React Context, TanStack Query	Reduces bundle size, improves empirical performance
Styling	Tailwind CSS + shadcn/ui; mobile-first, dark/light theming	Consistent branding, accessible design
Form Validation	React Hook Form + Zod schemas; RO-RO pattern for data flow	Guarantees end-to-end type safety
Data Processing	Delta tables (bronze/silver/gold), Spark SQL, dbt/GE tests	Observability into pipeline health
Vector Search	Pinecone index with versioned embeddings; monitor drift	Empirical feedback on model accuracy
CI/CD	Coverage, lint, performance budgets, canary deploys	Falsifiable gates before merging
Code Reviews	PR checklists: tests, types, docs, performance noted	Peer scrutiny prevents hidden assumptions
Documentation	docs/ARCHITECTURE.md, DECISIONS.md, autogenerated API specs	Reflective artifacts for onboarding and retrospectives

