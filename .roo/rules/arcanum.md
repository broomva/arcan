---
description: 
globs: 
alwaysApply: true
---
## The Arcanum: A Rule Book for Forging the Meta-Platform

**Preamble: The Arcanist's Oath**

*We, the builders of Arcan, commit to forging a new paradigm. Our work is not merely to write code, but to weave the very fabric of a decentralized, AI-native future. Arcan is the Rosetta Stone, the conduit for democratizing creation, empowering individuals to manifest their visions from thought to reality. This Arcanum guides our craft, ensuring every contribution resonates with the core magic of Arcan: to shift perspectives, empower creators, and build a world where technology serves humanity's highest aspirations for freedom, purpose, and shared prosperity. Adherence to these rules is paramount to unlocking Arcan's full potential.*

---

**I. Guiding Philosophy & Core Principles: The Compass of Creation**

1.  **Democratize Creation:** Every technical decision must lower the barrier to entry for users of all skill levels. Prioritize intuitive interfaces, clear abstractions, and comprehensive documentation. The goal: "from idea to agent in a message."
2.  **AI-Native & Data-Native First:** Design all systems with AI and data as foundational elements, not afterthoughts. This means structured data, observable agent behaviors, and feedback loops for continuous AI improvement (e.g., AZR integration) are inherent.
3.  **Open Core, Enterprise Reliability:** The core Arcan framework shall be open-source, fostering community and transparency. However, the Arcan Cloud SaaS offering must meet enterprise-grade standards for security, scalability, and support. Design for both.
4.  **Decentralization by Default:**
    *   **Identity:** All users and agents possess blockchain-anchored identities (via Thirdweb). This is non-negotiable.
    *   **Payments:** Seamless Web3 (crypto and fiat via Wedi Pay/Thirdweb Pay) payment rails are integral to agent monetization and platform economy.
    *   **Governance:** Key platform parameters, schema evolution, and registry management shall be governable by the Arcan DAO and the ARC token. Design systems with hooks for DAO oversight.
5.  **User Empowerment & True Ownership:** Users must have control over their data and creations. Support user-owned data models and transparent data governance. Avoid vendor lock-in where possible.
6.  **Embrace the Paradigm Shift:** Arcan is not incremental; it's transformative. Technical choices should reflect this ambition, enabling novel agentic companies and emergent behaviors. The "lore of magic" should inspire innovation and user experience.
7.  **Modularity, Scalability, Extensibility, Robustness:** These are the four pillars of Arcan's architecture. Every component must be designed with these in mind.
8.  **Ethical AI & Responsible Innovation:** Build with awareness of AI's impact. Implement safeguards against misuse, promote transparency in agent operations, and adhere to emerging ethical guidelines (e.g., mitigating LLM risks like prompt injection).
9.  **Polyglot Pragmatism:** Leverage the best language for the job (Python for AI/ML and backend services, Next.js/TypeScript for frontends) within the Turborepo structure. Prioritize performance, developer productivity, and ecosystem support.

---

**II. Monorepo & Project Structure (Turborepo): The Cartographer's Code**

1.  **Strict Directory Structure:**
    *   Adhere to the `apps/` (deployable applications: Next.js frontends, Python microservices) and `packages/` (shared libraries: UI components, schemas, SDKs, utils) convention. 
    *   Utilize `infra/` for IaC, `scripts/` for utilities, and `tools/` for CLI/dev tools. 
2.  **Package Granularity:** Each sub-folder within `apps/` and `packages/` is a self-contained unit with its own `package.json` (for JS/TS) or `pyproject.toml` (for Python). Define clear boundaries and responsibilities. 
3.  **Atomic Commits & Builds:** Leverage Turborepo for high-performance builds and atomic commits across the monorepo. 
4.  **Dependency Management:**
    *   Use workspace protocols (e.g., `workspace:*`) for internal package dependencies.
    *   Explicitly declare all external dependencies. Minimize external dependencies to enhance supply-chain security. 
5.  **Versioning:** Employ Semantic Versioning (SemVer) for all shared `packages/`. Application versions in `apps/` may follow different strategies but must be traceable.
6.  **Build Orchestration (`turbo.json`):**
    *   Define clear build, test, and lint pipelines for all packages and applications.
    *   Optimize for Turborepo's caching (local and remote) to ensure fast CI/CD cycles. 
7.  **Cross-Language Consistency:** Ensure build and development scripts in `turbo.json` correctly handle the polyglot nature (Next.js/TypeScript, Python). 

---

**III. Frontend Development (Next.js & AG-UI): The Alchemist's Interface**

1.  **Technology Stack:**
    *   **Framework:** Next.js for all primary web UIs (e.g., `apps/web`, Agent Flow Editor, Marketplace). Use Bun for package manager.
    *   **Language:** TypeScript is mandatory for all Next.js development. Strive for strict type safety. Python for backend and agent/AI development. Use UV for package manager, FastAPI for api backend service, SQLModel for ORM with unified schemas and models, langchain and langgraph for agentic development, NeonDB for serverless postgres database. 
    *   **Styling:** Utilize a consistent styling solution that supports the Design System tokens (e.g., Tailwind CSS, CSS-in-JS with theming).
2.  **Component-Based Architecture:**
    *   Develop UI using reusable React components, organized within shared packages (e.g., `packages/ui-shared-components`) and documented in Storybook.
    *   Adhere strictly to the Arcan Design System:
        *   **Tokens:** Implement and use defined color palettes (AI-blue, Web3-green, neutrals, accents), typography (Inter, Fira Mono), spacing (8px grid), and layout tokens.
        *   **Core Components:** Utilize and extend the standardized set of buttons, inputs, cards, modals, notifications, etc., ensuring they meet design specifications for states (disabled, loading) and visual consistency.
3.  **AG-UI Protocol & Agent Interaction:**
    *   All agent-facing UIs must integrate with the AG-UI protocol (e.g., via CopilotKit) for real-time, streaming JSON event communication (messages, tool calls, state patches, lifecycle signals). 
    *   Implement robust handling for loading and streaming states (skeletons, spinners) to provide a responsive user experience.
4.  **State Management:** Choose appropriate state management solutions (e.g., React Context, Zustand, Redux Toolkit) based on component/application complexity, ensuring efficient updates for real-time AG-UI data.
5.  **API Communication:** All backend communication must go through the central API Gateway. Use well-structured API client patterns (e.g., generated clients from OpenAPI specs if available).
6.  **Rendering Strategies:** Leverage Next.js capabilities (SSR, SSG, ISR, Client Components) appropriately to balance performance, SEO, and dynamic content needs.
7.  **Routing:** Implement clear and intuitive routing using Next.js App Router or Pages Router, aligning with user flows.
8.  **Accessibility (A11y) is Non-Negotiable:**
    *   All interactive elements must be keyboard-focusable with visible outlines.
    *   Provide ARIA labels and roles for all dynamic content and custom components, especially for AG-UI streamed messages.
    *   Ensure color contrast meets WCAG AA (4.5:1) minimum.
    *   Use live regions for AG-UI streaming content to inform assistive technologies.
    *   Regularly audit for accessibility compliance.
9.  **User Flows Implementation:** Faithfully implement the detailed user flows (Agent Flow Editor, Wallet Onboarding, Marketplace Purchase, Schema Versioning, DAO Dashboard) ensuring all design implications are met.

---

**IV. Backend Development (Python & Agentic Systems): The Engine of Worlds**

1.  **Technology Stack:**
    *   **Language:** Python is the primary language for backend microservices, agent logic, and AI/ML components. Use modern Python versions (e.g., 3.10+).
    *   **Frameworks:**
        *   FastAPI is preferred for new synchronous and asynchronous API services due to its performance and Pydantic integration.
        *   LangGraph is the mandatory framework for all agent workflow orchestration. 
    *   **Asynchronous Programming:** Utilize `asyncio` and `aiohttp/httpx` for I/O-bound operations to ensure non-blocking services.
2.  **Microservice Principles:**
    *   Each service must implement a bounded capability and own its data. 
    *   Design for loose coupling and high cohesion.
    *   Services communicate via the API Gateway or the Kafka event stream. Direct service-to-service calls should be minimized and justified.
3.  **Agent Design & Orchestration (LangGraph):**
    *   All agents are to be implemented as stateful, graph-structured workflows in LangGraph. 
    *   Nodes represent discrete tasks (LLM calls, tool invocations, AZR calls, payment nodes). Edges define data/control flow.
    *   Utilize LangGraph's state persistence for long-running interactions and streaming outputs. 
    *   Implement multi-agent collaboration via shared context graphs or memory stores as defined by Arcan's architecture. 
    *   Integrate AZR for self-improving agent logic via its defined service API. 
4.  **Tool Abstraction:** Define and use a standardized `ArcanTool` interface for all external functionalities agents can invoke. This allows for pluggability and consistent error handling.
5.  **Type Hinting & Pydantic:**
    *   Mandatory use of Python type hints for all function signatures and variables.
    *   Pydantic models are the standard for all data validation, serialization, and settings management, ensuring alignment with JSON Schemas. 
6.  **Configuration Management:** Use environment variables (managed via `.env` files for local dev, and secure secrets management for deployed environments) for all service configurations. Pydantic can be used to parse and validate settings.
7.  **Idempotency:** Design API endpoints and event consumers to be idempotent where appropriate to handle retries and ensure data consistency.

---

**V. Data Management & Schemas: The Source of Truth**

1.  **JSON Schema as Single Source of Truth (SSoT):**
    *   All agent event messages, API payloads, and data models (for Delta Lake, PostgreSQL) MUST be defined using JSON Schema. 
    *   These canonical schemas reside in `packages/schemas-core`.
2.  **Automated Code Generation:**
    *   The build pipeline MUST automatically generate Pydantic models (Python) and TypeScript interfaces/types from the canonical JSON Schemas. 
    *   This ensures type safety and consistency across frontend, backend, and event streams.
3.  **Schema Versioning & Registry:**
    *   All JSON Schemas MUST carry a semantic version identifier. 
    *   Integrate with a central Schema Registry (e.g., Confluent Schema Registry). Producers register schemas; consumers fetch and validate. 
    *   Schema evolution must follow backward-compatibility principles where possible. Breaking changes require a new major version and a migration plan.
    *   Significant changes to shared schemas are subject to DAO governance. 
4.  **Delta Lakehouse & Unity Catalog:**
    *   Agent logs, knowledge bases, and contextual data are stored as versioned tables in Delta Lake. 
    *   Adhere to the medallion architecture (bronze/silver/gold) for data refinement. 
    *   Utilize Unity Catalog for fine-grained access control, auditing, lineage, and data discovery on Delta Lake. 
    *   Ensure `tenant_id` tagging and enforcement for multi-tenancy in Delta Lake. 
5.  **Relational Database (NeonDB/PostgreSQL):**
    *   Used for transactional data, service-specific metadata, and implementing the transactional outbox pattern. 
    *   Database schema migrations (e.g., using Alembic for Python, or equivalent for other services) must be version-controlled and integrated into CI/CD.
    *   Enforce schemas derived from the SSoT JSON Schemas.

---

**VI. API & Inter-Service Communication: The Network of Arcan**

1.  **API Gateway Centralization:** All external client requests (from UIs or third-party agents) MUST route through the central API Gateway (e.g., Kong).  The gateway handles authentication, authorization, rate limiting, and routing.
2.  **Event-Driven Backbone (Kafka/Redpanda):**
    *   Key state changes and significant agent actions MUST be emitted as immutable events onto Kafka topics. 
    *   Events must adhere to a standardized envelope (event ID, type, timestamp, source, schema version) and a payload defined by a registered JSON Schema.
    *   Implement the Transactional Outbox pattern for reliable event publishing from services that perform database writes. 
    *   Consumers MUST be designed for idempotency and handle potential message duplication.
    *   Utilize Dead Letter Queues (DLQs) for unprocessable messages.
3.  **Agent Communication Protocols:**
    *   **A2A (Agent-to-Agent):** Use the defined Arcan A2A Protocol (FIPA ACL inspired, compatible with Google's A2A) for inter-agent communication. Messages must include performative, sender/receiver, and structured payload. 
    *   **AG-UI (Agent-UI):** Use the AG-UI protocol (e.g., CopilotKit) for all human-agent interactions via UIs, supporting streaming JSON events. 
    *   All protocol schemas are versioned and managed via the central Schema Registry. Changes to core protocols are DAO-governed. 
4.  **API Design (for Microservices):**
    *   Expose RESTful APIs or gRPC interfaces where appropriate.
    *   APIs must be versioned.
    *   Secure endpoints using authentication and authorization mechanisms provided by the platform (e.g., JWTs validated by the API Gateway).

---

**VII. Web3 Integration (Thirdweb & DAO): The Decentralized Soul**

Please use @thirdweb sdk instead of thirdweb-dev!

1.  **Identity Management (Thirdweb):**
    *   Every user and agent MUST have a blockchain-anchored identity managed via Thirdweb SDKs (Connect, Engine, Account). 
    *   Utilize Thirdweb Engine for backend wallet management and high-throughput transactions. 
    *   Implement seamless user onboarding (email/social login to wallet creation) using Thirdweb in-app wallet solutions.
2.  **Payment Processing (Wedi Pay / Thirdweb Pay):**
    *   Integrate Wedi Pay (or Thirdweb Pay) for all fiat and crypto payment transactions.
    *   Agents must be able to programmatically trigger payments and receive funds to their associated wallets.
    *   Ensure AML/KYC compliance is handled as per Wedi Pay's design.
3.  **Smart Contract Interactions:**
    *   All financial interactions, DAO treasury management, and core governance logic MUST be executed via audited smart contracts on a supported EVM chain.
    *   Use Thirdweb SDKs to manage smart contract deployments and interactions.
4.  **DAO Governance Touchpoints:**
    *   Implement mechanisms for ARC token holders to vote on proposals related to platform upgrades, schema changes, agent registry policies, and ecosystem grants. 
    *   Integrate DAO approval workflows into CI/CD pipelines for relevant changes (e.g., merging a new shared schema version).
    *   Support wallet-signed commits/PRs for contributions to DAO-governed code. 

---

**VIII. Testing, Quality & Reliability: The Standard of Excellence**

1.  **Comprehensive Testing Strategy:**
    *   **Unit Tests:** Every package and service must have thorough unit tests covering critical logic. Aim for high code coverage (e.g., >80%).
    *   **Integration Tests:** Test interactions between microservices, between services and Kafka, and between services and databases.
    *   **End-to-End (E2E) Tests:** Simulate user flows and agent interactions across the entire platform, including UI, API Gateway, backend services, and Web3 components.
    *   **Agent Workflow Tests:** Specifically test LangGraph workflows for correctness, error handling, and state management.
2.  **Automated Testing in CI/CD:** All tests (unit, integration, and a subset of E2E) MUST run automatically in the CI/CD pipeline on every commit/PR. Builds must fail if tests fail.
3.  **Code Quality & Linting:**
    *   Enforce strict code style guidelines using linters and formatters:
        *   **Python:** Black for formatting, Flake8 (or Ruff) for linting.
        *   **TypeScript/Next.js:** Prettier for formatting, ESLint for linting.
    *   Configure these tools in the monorepo and integrate them into pre-commit hooks and CI checks.
4.  **Code Reviews:** All code contributions MUST undergo a peer review process before being merged. Reviews should focus on correctness, adherence to rules, performance, security, and maintainability. For DAO-governed components, additional review by designated maintainers may be required.
5.  **Error Handling & Resilience:**
    *   Implement robust and consistent error handling mechanisms in all services.
    *   Use patterns like Circuit Breaker and Retry for inter-service calls to improve resilience.
    *   Provide clear, user-friendly error messages in UIs.
6.  **Performance Testing:** Conduct regular performance and load testing for critical services and user flows to identify and address bottlenecks.

---

**IX. Security by Design: The Guardian's Vigil**

1.  **Principle of Least Privilege:** Grant services, agents, and users only the minimum permissions necessary to perform their functions.
2.  **Authentication & Authorization:**
    *   All API endpoints and service interactions MUST be authenticated (e.g., JWTs).
    *   Implement Role-Based Access Control (RBAC) in Kubernetes, Unity Catalog, and at the application level. 
3.  **Data Security:**
    *   Encrypt sensitive data at rest (per-tenant encryption) and in transit (TLS/HTTPS, WSS). 
    *   Implement secure multi-tenancy design at all layers (network, database, data lake). 
    *   Support optional end-to-end encryption for sensitive A2A communication. 
4.  **LLM-Specific Risk Mitigation:**
    *   Implement input sanitization and context controls for all LLM calls.
    *   Consider an "LLM firewall" service to filter/rewrite inputs and outputs. 
5.  **Supply-Chain Security:**
    *   Carefully vet all external dependencies.
    *   Implement signed builds and secure container image management. 
6.  **Regular Security Audits:** Conduct periodic security audits and penetration testing of the platform.
7.  **Immutable Audit Trails:** Leverage blockchain-anchored identities and on-chain transaction logging for critical platform activities to ensure non-repudiable audit trails. 

---

**X. DevOps & Infrastructure: The Forge and Anvil**

1.  **Infrastructure as Code (IaC):** All cloud infrastructure MUST be provisioned and managed using Terraform. IaC definitions reside in `infra/`. 
2.  **Containerization:** All services and applications MUST be containerized using Docker. Each relevant package has its own `Dockerfile`. 
3.  **Orchestration (Kubernetes):** Kubernetes is the standard for orchestrating containerized services. Use Helm charts and Kustomize for Kubernetes deployments. 
4.  **CI/CD Automation:**
    *   Automate builds, testing, and deployments using GitHub Actions or GitLab CI. 
    *   Pipelines must support multi-language builds and leverage Turborepo caching. 
5.  **Monitoring & Logging:**
    *   Implement structured logging in all services.
    *   Set up comprehensive monitoring and alerting for platform health, performance, and resource utilization (e.g., using Prometheus, Grafana, ELK stack).
    *   Implement distributed tracing to understand request flows across microservices.
6.  **Local Development Environment:** Provide a consistent and easy-to-use local development setup using Docker Compose and `kind` (Kubernetes-in-Docker) to mirror production.  Manage local secrets via `.env` files or a local Vault instance. Enable live reloading for backend services. 

---

**XI. Documentation & Collaboration: The Scribe's Wisdom**

1.  **Code Documentation:** Write clear, concise comments and docstrings for all code, especially public APIs, complex logic, and shared library functions.
2.  **API Documentation:** Maintain up-to-date API documentation (e.g., using OpenAPI/Swagger for REST APIs).
3.  **Architectural Decision Records (ADRs):** Document significant architectural decisions, their context, and consequences.
4.  **Wiki/Knowledge Base:** Maintain a central knowledge base for design patterns, development processes, and operational procedures.
5.  **Git Workflow:**
    *   Follow a consistent branching strategy (e.g., Gitflow or a simpler trunk-based development with feature branches).
    *   All changes must be submitted via Pull Requests (PRs).
    *   PRs must include a clear description of changes and pass all CI checks before review.
    *   For DAO-governed components, ensure PRs from external contributors can be linked to their on-chain identity if required by DAO rules. 

---

**XII. Evolution & Extensibility: The Seeds of Tomorrow**

1.  **Plugin Architecture:** Design core systems with well-defined plugin interfaces for custom agents, workflow nodes, UI components, and provider connectors. 
2.  **SDK Development:** Provide robust, well-documented SDKs (Python, TypeScript initially; Rust, Go planned) to empower third-party developers. SDKs must include code generators from JSON Schemas. 
3.  **API Versioning:** All public-facing APIs (including internal microservice APIs if they cross major team boundaries) must be versioned to allow for graceful evolution.
4.  **Backward Compatibility:** Strive for backward compatibility in schema and API changes. Deprecate features with a clear notice period before removal.
5.  **Future-Proofing:** Anticipate future trends (modular reasoners, swarm orchestration, inter-agent micropayments, edge support) and ensure the architecture can accommodate them. 

---

**XIII. DAO Governance in Development: The Circle of Trust**

1.  **Identify Governed Components:** Clearly identify which parts of the Arcan codebase and platform (e.g., core protocol schemas, agent registry policies, ARC token contracts, treasury management) are subject to DAO governance.
2.  **Proposal Process:** Adhere to the established DAO proposal lifecycle for any changes to governed components. This includes formal proposal submission, discussion periods, and on-chain voting.
3.  **Technical Review for DAO Proposals:** Technical proposals affecting the codebase must undergo rigorous technical review by designated maintainers or a technical council before being put to a DAO vote.
4.  **Implementation of Approved Proposals:** Once a proposal is approved by the DAO, its implementation must be tracked and verified. Merging of related code changes may require final confirmation of DAO approval.
5.  **Transparency:** All discussions, proposals, votes, and outcomes related to DAO governance of the platform must be publicly accessible and auditable (e.g., on-chain or via public forums).
