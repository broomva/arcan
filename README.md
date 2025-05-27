# Arcan ✨ - The AI-Native Meta-Platform

**Arcan is the open, AI-native meta-platform that lets anyone "build, deploy, and monetize" full-stack AI applications (agents) with seamless Web3 identity and payments. It's governed by a DAO yet delivered by a dedicated SaaS company—melding openness with enterprise reliability.**

_From idea to agent in a message. Arcan is the Rosetta Stone for a new era of creation, a conduit for democratizing purpose and empowering everyone to build what they love._

## 📜 Overview

Arcan is more than a platform; it's a **paradigm shift**. We are building a comprehensive ecosystem designed to democratize AI creation and empower individuals and businesses to forge their own "agentic companies." By seamlessly fusing advanced AI orchestration, robust enterprise-grade data infrastructure, and natively integrated blockchain-based identity, payments, and governance, Arcan aims to unlock unprecedented levels of innovation and user agency.

Our philosophy is rooted in the belief that technology should empower, not control. Inspired by a vision to move beyond techno-feudalism, Arcan offers tools for true ownership, decentralized collaboration, and the realization of individual purpose. This is where the deep magic of AI meets the liberating potential of Web3, allowing anyone—from an indie hacker to an enterprise, from your mother to a seasoned developer—to create, monetize, and thrive.

Arcan is the core from which new agentic enterprises like **Wedi** (for seamless Web3 payments) emerge, each inheriting the power of the meta-platform to address unique challenges and create value.

## 🚀 Core Features

Arcan provides a rich set of capabilities to bring your AI-driven visions to life:

- **🤖 Agent Creation & Orchestration:** Visually design, build, and deploy sophisticated AI agents using **LangGraph**. Leverage pre-built templates (e.g., "Travel Agent," "Invoice Generator") or create custom workflows with self-improving logic via **AZR (Absolute Zero Reasoner)** integration.
- **🎨 Multimodal UI (AG-UI):** Engage with agents through dynamic, real-time UIs powered by a unified JSON event stream (e.g., via **CopilotKit**). Build interactive experiences with embeddable React components adhering to the Arcan Design System.
- **🔗 A2A Agent Gateway:** Enable secure and discoverable communication between agents using standardized protocols (FIPA ACL-inspired, Google A2A). Manage trust and permissions for external agent interactions.
- **💾 Data & State Management:** Utilize a unified **JSON Schema** registry for all data models, ensuring consistency across a **Delta Lakehouse** backend with **Unity Catalog** governance. Implement schema-driven forms and validation.
- **💸 Payments & Wallets (Wedi Pay & Thirdweb):** Seamlessly integrate Web3 identity (**Thirdweb SDK & Engine**) and payment rails (**Wedi Pay** for fiat/crypto, **Thirdweb Pay**) into your agents for monetization and transactions. Facilitate easy wallet onboarding (email/social login to wallet).
- **🛍️ Agent Marketplace:** Publish, discover, and subscribe to agents. Define flexible pricing models (subscription, one-off, pay-per-use) and monitor performance via a dedicated sales dashboard.
- **🏛️ DAO Governance & ARC Token:** Participate in the decentralized governance of the Arcan platform through the **ARC token**. Influence protocol evolution, schema management, agent registry policies, and ecosystem funding via on-chain voting.

## ✨ Guiding Philosophy: The Arcanist's Creed

- **Democratize Creation:** Lowering barriers for everyone. "Create at the command of a thought."
- **AI-Native & Data-Native First:** AI and structured data are foundational, not afterthoughts.
- **Open Core, Enterprise Reliability:** Fostering community with dependable SaaS offerings.
- **Decentralization by Default:** Empowering users with Web3 identity, payments, and true DAO governance.
- **User Empowerment & True Ownership:** Resisting techno-feudalism; users control their data and creations.
- **Embrace the Paradigm Shift:** Enabling transformative agentic companies and a new way of interacting with technology and economy.
- **Inspired by Lore & Magic:** Infusing the platform with a sense of wonder, possibility, and the power to change the world.

## 🛠️ Technology Stack

Arcan leverages a modern, polyglot technology stack for performance, scalability, and flexibility:

- **Monorepo Management:** Turborepo
- **Frontend:** Next.js (TypeScript), React, AG-UI (e.g., CopilotKit)
- **Backend & AI:** Python (FastAPI, LangGraph), Absolute Zero Reasoner (AZR)
- **Event Streaming:** Apache Kafka / Redpanda
- **Data Lakehouse:** Delta Lake, Unity Catalog (on Databricks)
- **Relational Database:** PostgreSQL (e.g., NeonDB for transactional outbox)
- **Containerization & Orchestration:** Docker, Kubernetes
- **Web3 Integration:** Thirdweb SDK & Engine, Solidity (for Smart Contracts), Wedi Pay
- **API Gateway:** Kong / AWS API Gateway (examples)
- **Infrastructure as Code:** Terraform, Helm, Kustomize
- **CI/CD:** GitHub Actions / GitLab CI (examples)

## 📂 Project Structure (Turborepo Monorepo)

Arcan's codebase is meticulously organized within a Turborepo monorepo to manage its polyglot nature and ensure high-performance, atomic builds:

- `apps/`: Contains deployable applications.
  - `frontend-main`: The primary web UI for Arcan.
  - _Other Next.js frontends (e.g., Agent Flow Editor, Marketplace UI)._
  - _Python backend microservices (e.g., `payment-service`, `agent-orchestration-service`, `azr-integration-service`)._
- `packages/`: Houses shared libraries, utilities, and configurations.
  - `schemas-core`: Centralized JSON Schema definitions (the single source of truth).
  - `ui-shared-components`: Reusable React UI components (Arcan Design System, Storybook).
  - `agent-python-sdk`: Core SDK for Python agent development (LangGraph nodes, tool interfaces).
  - `agent-typescript-sdk`: Core SDK for TypeScript agent/UI development (AG-UI clients).
  - `common-utils`: Shared utility functions, constants, and helper libraries.
  - `arcan/`: Python package for the Arcan spellbook
- `infra/`: Infrastructure-as-Code definitions (Terraform for cloud resources, Helm charts for Kubernetes).
- `scripts/`: Utility scripts for development, deployment, and operational tasks.
- `tools/`: CLI tools and developer utilities specific to Arcan.
- `contracts/`: Solidity smart contracts for DAO governance, ARC token, payment logic, etc.

Each sub-folder is a self-contained package with its own `package.json` (JS/TS) or `pyproject.toml` (Python), promoting modularity and independent development cycles.

## 🚀 Getting Started

Welcome, Arcanist\! To begin your journey with Arcan:

1.  \*\*Clone the Repository:\*\*bash
    git clone [https://github.com/your-org/arcan.git](https://www.google.com/search?q=https://github.com/your-org/arcan.git)
    cd arcan
    ```

    ```
2.  **Install Dependencies:** (Turborepo handles workspace linking)
    ```bash
    npm install # or yarn install
    ```
3.  **Local Development Environment:**
    - Spin up essential services (PostgreSQL, Kafka, Redis, MinIO for S3 mock) using Docker Compose:
      ```bash
      docker-compose up -d
      ```
    - Configure local environment variables by copying `.env.example` files in respective packages/apps to `.env.local` and customizing them.
4.  **Run an Application (Example: Main Frontend):**
    ```bash
    turbo run dev --filter=@arcan/frontend-main
    ```
5.  **Explore Packages:** Each package in `packages/` and application in `apps/` has its own README with specific instructions.

For a more comprehensive guide on local setup, including Kubernetes emulation with `kind`, advanced configurations, and contribution workflows, please refer to our `DEVELOPMENT_GUIDE.md`.

## 🏛️ Governance (Arcan DAO)

Arcan is not just built _for_ the community; it is governed _by_ the community. The **Arcan DAO**, powered by the **ARC token**, is the heart of our decentralized governance model.

- **ARC Token Utility:** ARC tokens grant voting power in DAO decisions, and may be used for staking (e.g., to publish agents, secure network operations) or accessing premium features.
- **On-Chain Governance:** Proposals regarding protocol upgrades, shared schema evolution, Agent Registry policies, treasury management, and ecosystem grants are submitted, discussed, and voted upon on-chain.
- **Transparency:** All governance processes are designed to be transparent and auditable. Smart contracts defining DAO rules and schema versions are open-source and verifiable.
- **Inspired by Proven Models:** Arcan's DAO structure draws inspiration from successful frameworks like Aragon and Juicebox for robust treasury management and permissioned operations.

Join the discussion and help shape the future of Arcan\! (Link to governance forum/portal to be added).

## 🤝 Contributing

Arcan is a collective endeavor. We believe in the power of community to build something truly revolutionary. Whether you're a developer, designer, AI researcher, Web3 enthusiast, writer, or visionary, your contributions are invaluable.

Please read our `CONTRIBUTING.md` for details on:

- Our Code of Conduct.
- The development workflow (including Git practices and PR submissions).
- How to set up your environment for contributing.
- Areas where you can help (e.g., core platform features, SDKs, new agents, UI components, documentation, DAO proposals).
- Wallet-signed commits for contributions to DAO-governed code.

## 🔮 Future Vision: The Unfolding Meta-Platform

Arcan is architected for evolution, designed to be the universal "agent development kit" and the bedrock for a new generation of AI-driven enterprises. Our roadmap includes:

- **Modular and Swappable Reasoning Engines:** Expanding beyond AZR to integrate diverse AI reasoning paradigms, allowing an agent's "brain" to be updated or ensembled.
- **Swarm Orchestration:** Enabling sophisticated, dynamic coordination among vast populations of agents, including ephemeral sub-agents and geographically distributed operations.
- **Inter-Agent Token Micropayments:** Facilitating a true agent economy where agents can autonomously "rent" compute, data, or services from each other using ARC tokens or stablecoins.
- **Edge and Offline Agent Support:** Extending Arcan's capabilities to on-device agents (mobile, IoT) that can operate with intermittent connectivity, leveraging federated learning and distributed caches.

We envision Arcan as the catalyst for a new wave of platform-enabled entrepreneurship, making it possible for anyone, anywhere, to transform their ideas into impactful, autonomous, and decentralized businesses with unprecedented ease and freedom.

## 📜 License

The core Arcan framework and its associated open-source components are typically licensed under the **Apache 2.0 License** or a similar permissive license. We are committed to fostering an open and collaborative ecosystem.

Please refer to the `LICENSE` file in the root directory and individual `LICENSE` files within specific packages for detailed licensing information.

---

_The Arcanum is open. The future is agentic. Join us in weaving the next reality._
