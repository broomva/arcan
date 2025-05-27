# Arcan Implementation Roadmap - Phase 1

## Week 1: Schema Management Foundation

### Day 1-2: Core Schemas Package Setup

1. **Create Package Structure**

```bash
mkdir -p packages/core-schemas/{json,python,typescript,scripts}
mkdir -p packages/core-schemas/json/{common,agent,event,workflow,finance}
```

2. **Initialize Package Configuration**

- Create `package.json` for TypeScript tooling
- Create `pyproject.toml` for Python tooling
- Add to Turborepo pipeline

3. **Install Code Generation Tools**

```json
// packages/core-schemas/package.json
{
  "name": "@arcan/core-schemas",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "clean": "rm -rf ./python/models/* ./typescript/*",
    "generate:python": "node scripts/generate-python.js",
    "generate:ts": "json-schema-to-typescript 'json/**/*.json' -o typescript/",
    "build": "npm run clean && npm run generate:python && npm run generate:ts",
    "validate": "ajv validate -s json/meta-schema.json -d 'json/**/*.json'"
  },
  "devDependencies": {
    "json-schema-to-typescript": "^13.1.2",
    "ajv-cli": "^5.0.0",
    "datamodel-code-generator": "^0.25.0"
  }
}
```

### Day 3-4: Define Core JSON Schemas

1. **Common Types Schema**

```json
// packages/core-schemas/json/common/types.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://arcan.ai/schemas/common/types/v1.0.0",
  "definitions": {
    "UUID": {
      "type": "string",
      "format": "uuid",
      "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
    },
    "Timestamp": {
      "type": "string",
      "format": "date-time"
    },
    "TenantId": {
      "$ref": "#/definitions/UUID"
    }
  }
}
```

2. **Agent Event Schema**

```json
// packages/core-schemas/json/event/agent-interaction.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://arcan.ai/schemas/event/agent-interaction/v1.0.0",
  "title": "AgentInteractionEvent",
  "type": "object",
  "properties": {
    "eventId": { "$ref": "../common/types.json#/definitions/UUID" },
    "tenantId": { "$ref": "../common/types.json#/definitions/TenantId" },
    "agentId": { "type": "string" },
    "timestamp": { "$ref": "../common/types.json#/definitions/Timestamp" },
    "eventType": {
      "type": "string",
      "enum": ["message.received", "tool.called", "state.updated", "error.occurred"]
    },
    "payload": { "type": "object" }
  },
  "required": ["eventId", "tenantId", "agentId", "timestamp", "eventType", "payload"]
}
```

### Day 5: Code Generation Pipeline

1. **Python Generation Script**

```javascript
// packages/core-schemas/scripts/generate-python.js
const { exec } = require('child_process');
const fs = require('fs');
const path = require('path');

const generatePython = async () => {
  const jsonDir = path.join(__dirname, '../json');
  const outputDir = path.join(__dirname, '../python/models');

  // Ensure output directory exists
  fs.mkdirSync(outputDir, { recursive: true });

  // Generate Pydantic models
  const cmd = `datamodel-codegen --input ${jsonDir} --output ${outputDir} --use-default --target-python-version 3.10`;

  exec(cmd, (error, stdout, stderr) => {
    if (error) {
      console.error(`Error: ${error}`);
      return;
    }
    console.log('Python models generated successfully');
  });
};

generatePython();
```

## Week 2: Event-Driven Architecture

### Day 1-2: Event Streaming Package

1. **Create Event Streaming Package**

```bash
mkdir -p packages/event-streaming/{src,tests}
```

2. **Base Event Publisher**

```python
# packages/event-streaming/src/publisher.py
from typing import Any, Dict, Optional
from abc import ABC, abstractmethod
import json
import asyncio
from aiokafka import AIOKafkaProducer
from pydantic import BaseModel
import logging

logger = logging.getLogger(__name__)

class EventPublisher(ABC):
    """Abstract base class for event publishers"""

    @abstractmethod
    async def publish(self, topic: str, event: BaseModel) -> None:
        """Publish an event to a topic"""
        pass

class KafkaEventPublisher(EventPublisher):
    """Kafka implementation of event publisher"""

    def __init__(self, bootstrap_servers: str, **kwargs):
        self.bootstrap_servers = bootstrap_servers
        self.producer: Optional[AIOKafkaProducer] = None
        self.kwargs = kwargs

    async def start(self):
        """Start the Kafka producer"""
        self.producer = AIOKafkaProducer(
            bootstrap_servers=self.bootstrap_servers,
            value_serializer=lambda v: json.dumps(v).encode(),
            **self.kwargs
        )
        await self.producer.start()
        logger.info("Kafka producer started")

    async def stop(self):
        """Stop the Kafka producer"""
        if self.producer:
            await self.producer.stop()
            logger.info("Kafka producer stopped")

    async def publish(self, topic: str, event: BaseModel) -> None:
        """Publish an event to Kafka"""
        if not self.producer:
            raise RuntimeError("Producer not started")

        try:
            await self.producer.send_and_wait(
                topic,
                value=event.model_dump(mode='json')
            )
            logger.debug(f"Published event to {topic}: {event.model_dump_json()}")
        except Exception as e:
            logger.error(f"Failed to publish event: {e}")
            raise
```

### Day 3-4: Transactional Outbox Implementation

1. **Outbox Models**

```python
# packages/event-streaming/src/outbox.py
from sqlmodel import SQLModel, Field, Session
from typing import Optional, Dict, Any
from datetime import datetime
from uuid import UUID, uuid4
import enum

class OutboxStatus(str, enum.Enum):
    PENDING = "PENDING"
    PUBLISHED = "PUBLISHED"
    FAILED = "FAILED"

class OutboxEvent(SQLModel, table=True):
    """Transactional outbox for reliable event publishing"""

    __tablename__ = "transactional_outbox"

    id: UUID = Field(default_factory=uuid4, primary_key=True)
    aggregate_id: str = Field(index=True)
    event_type: str
    topic: str
    payload: Dict[str, Any] = Field(sa_column_kwargs={"type": "JSONB"})
    created_at: datetime = Field(default_factory=datetime.utcnow)
    published_at: Optional[datetime] = None
    status: OutboxStatus = Field(default=OutboxStatus.PENDING)
    retry_count: int = Field(default=0)
    error_message: Optional[str] = None
```

2. **Outbox Publisher Service**

```python
# packages/event-streaming/src/outbox_publisher.py
import asyncio
from typing import List
from sqlalchemy.ext.asyncio import AsyncSession
from sqlmodel import select
from datetime import datetime, timedelta
import logging

from .outbox import OutboxEvent, OutboxStatus
from .publisher import EventPublisher

logger = logging.getLogger(__name__)

class OutboxPublisherService:
    """Service to publish events from the transactional outbox"""

    def __init__(
        self,
        session_factory,
        event_publisher: EventPublisher,
        batch_size: int = 100,
        max_retries: int = 3,
        retry_delay_seconds: int = 60
    ):
        self.session_factory = session_factory
        self.event_publisher = event_publisher
        self.batch_size = batch_size
        self.max_retries = max_retries
        self.retry_delay_seconds = retry_delay_seconds
        self._running = False

    async def start(self):
        """Start the outbox publisher"""
        self._running = True
        await self.event_publisher.start()

        while self._running:
            try:
                await self._process_batch()
                await asyncio.sleep(1)  # Poll interval
            except Exception as e:
                logger.error(f"Error processing outbox batch: {e}")
                await asyncio.sleep(5)  # Error backoff

    async def stop(self):
        """Stop the outbox publisher"""
        self._running = False
        await self.event_publisher.stop()

    async def _process_batch(self):
        """Process a batch of pending events"""
        async with self.session_factory() as session:
            # Query for pending events
            retry_threshold = datetime.utcnow() - timedelta(seconds=self.retry_delay_seconds)

            statement = select(OutboxEvent).where(
                (OutboxEvent.status == OutboxStatus.PENDING) |
                ((OutboxEvent.status == OutboxStatus.FAILED) &
                 (OutboxEvent.retry_count < self.max_retries) &
                 (OutboxEvent.published_at < retry_threshold))
            ).limit(self.batch_size)

            events = await session.execute(statement)
            events = events.scalars().all()

            for event in events:
                await self._publish_event(session, event)

            await session.commit()

    async def _publish_event(self, session: AsyncSession, event: OutboxEvent):
        """Publish a single event"""
        try:
            # Create a Pydantic model from the payload
            # In real implementation, you'd deserialize based on event_type
            await self.event_publisher.publish(event.topic, event.payload)

            # Mark as published
            event.status = OutboxStatus.PUBLISHED
            event.published_at = datetime.utcnow()
            logger.info(f"Published event {event.id} to {event.topic}")

        except Exception as e:
            # Mark as failed
            event.status = OutboxStatus.FAILED
            event.retry_count += 1
            event.error_message = str(e)
            event.published_at = datetime.utcnow()
            logger.error(f"Failed to publish event {event.id}: {e}")
```

### Day 5: Integration with API Service

1. **Update API Service Dependencies**

```toml
# apps/api/pyproject.toml
[project]
dependencies = [
    # ... existing dependencies ...
    "aiokafka>=0.10.0",
    "@arcan/event-streaming @ file:///../../packages/event-streaming",
]
```

2. **Add Outbox to Agent Service**

```python
# apps/api/src/db/services/agent.py
from sqlalchemy.ext.asyncio import AsyncSession
from packages.event_streaming.src.outbox import OutboxEvent
import json

class AgentService:
    async def create(self, agent: AgentCreate, tenant_id: UUID) -> Agent:
        """Create a new agent with event publishing"""
        async with self.session.begin():
            # Create agent
            db_agent = Agent(
                **agent.model_dump(),
                tenant_id=tenant_id,
            )
            self.session.add(db_agent)
            await self.session.flush()

            # Create outbox event
            event_payload = {
                "agent_id": str(db_agent.id),
                "tenant_id": str(tenant_id),
                "name": db_agent.name,
                "type": db_agent.type,
                "status": db_agent.status,
                "created_at": db_agent.created_at.isoformat()
            }

            outbox_event = OutboxEvent(
                aggregate_id=str(db_agent.id),
                event_type="agent.created.v1",
                topic="arcan.agents",
                payload=event_payload
            )
            self.session.add(outbox_event)

        return db_agent
```

## Week 3: Data Platform Foundation

### Day 1-2: Delta Lake Setup

1. **Create Data Platform Package**

```bash
mkdir -p packages/data-platform/{src,tests,notebooks}
```

2. **Delta Lake Configuration**

```python
# packages/data-platform/src/config.py
from pydantic_settings import BaseSettings
from typing import Optional

class DataPlatformSettings(BaseSettings):
    """Configuration for the data platform"""

    # Storage
    storage_account_name: str
    storage_container_name: str = "arcan-lakehouse"

    # Databricks
    databricks_host: Optional[str] = None
    databricks_token: Optional[str] = None

    # Delta Lake paths
    bronze_path: str = "bronze"
    silver_path: str = "silver"
    gold_path: str = "gold"

    # Unity Catalog
    catalog_name: str = "arcan_catalog"

    class Config:
        env_prefix = "ARCAN_DATA_"
```

3. **Delta Lake Writer**

```python
# packages/data-platform/src/delta_writer.py
from delta import DeltaTable, configure_spark_with_delta_pip
from pyspark.sql import SparkSession
from pyspark.sql.types import StructType
from typing import Dict, Any, Optional
import logging

logger = logging.getLogger(__name__)

class DeltaLakeWriter:
    """Writer for Delta Lake tables"""

    def __init__(self, spark: SparkSession, base_path: str):
        self.spark = spark
        self.base_path = base_path

    @classmethod
    def create_spark_session(cls, app_name: str = "ArcanDataPlatform") -> SparkSession:
        """Create a Spark session configured for Delta Lake"""
        builder = SparkSession.builder \
            .appName(app_name) \
            .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension") \
            .config("spark.sql.catalog.spark_catalog", "org.apache.spark.sql.delta.catalog.DeltaCatalog")

        spark = configure_spark_with_delta_pip(builder).getOrCreate()
        return spark

    def write_to_bronze(
        self,
        data: Dict[str, Any],
        table_name: str,
        partition_cols: Optional[list] = None
    ):
        """Write raw data to bronze layer"""
        path = f"{self.base_path}/bronze/{table_name}"

        df = self.spark.createDataFrame([data])

        writer = df.write.format("delta").mode("append")

        if partition_cols:
            writer = writer.partitionBy(*partition_cols)

        writer.save(path)
        logger.info(f"Written data to bronze layer: {path}")

    def create_silver_table(
        self,
        bronze_table: str,
        silver_table: str,
        transformation_sql: str
    ):
        """Create or update silver table from bronze"""
        bronze_path = f"{self.base_path}/bronze/{bronze_table}"
        silver_path = f"{self.base_path}/silver/{silver_table}"

        # Read bronze data
        bronze_df = self.spark.read.format("delta").load(bronze_path)
        bronze_df.createOrReplaceTempView("bronze_data")

        # Apply transformation
        silver_df = self.spark.sql(transformation_sql)

        # Write to silver
        silver_df.write.format("delta").mode("overwrite").save(silver_path)
        logger.info(f"Created/updated silver table: {silver_path}")
```

### Day 3-4: Unity Catalog Integration

1. **Unity Catalog Manager**

```python
# packages/data-platform/src/unity_catalog.py
from databricks.sdk import WorkspaceClient
from databricks.sdk.service.catalog import CreateCatalog, CreateSchema
from typing import List, Optional
import logging

logger = logging.getLogger(__name__)

class UnityCatalogManager:
    """Manager for Unity Catalog operations"""

    def __init__(self, workspace_client: WorkspaceClient):
        self.client = workspace_client
        self.catalog_api = workspace_client.catalogs
        self.schemas_api = workspace_client.schemas
        self.tables_api = workspace_client.tables
        self.grants_api = workspace_client.grants

    async def setup_catalog(self, catalog_name: str):
        """Set up the Arcan catalog"""
        try:
            # Create catalog if not exists
            self.catalog_api.create(name=catalog_name)
            logger.info(f"Created catalog: {catalog_name}")
        except Exception as e:
            if "already exists" in str(e):
                logger.info(f"Catalog already exists: {catalog_name}")
            else:
                raise

        # Create schemas
        schemas = ["bronze", "silver", "gold", "ml_features"]
        for schema in schemas:
            await self.create_schema(catalog_name, schema)

    async def create_schema(self, catalog_name: str, schema_name: str):
        """Create a schema in the catalog"""
        full_name = f"{catalog_name}.{schema_name}"
        try:
            self.schemas_api.create(
                name=schema_name,
                catalog_name=catalog_name
            )
            logger.info(f"Created schema: {full_name}")
        except Exception as e:
            if "already exists" in str(e):
                logger.info(f"Schema already exists: {full_name}")
            else:
                raise

    async def grant_permissions(
        self,
        principal: str,
        object_type: str,
        object_name: str,
        privileges: List[str]
    ):
        """Grant permissions on catalog objects"""
        for privilege in privileges:
            self.grants_api.update(
                principal=principal,
                securable_type=object_type,
                full_name=object_name,
                changes=[{"add": [privilege]}]
            )
        logger.info(f"Granted {privileges} on {object_name} to {principal}")
```

### Day 5: Event to Delta Lake Pipeline

1. **Kafka to Delta Streaming**

```python
# packages/data-platform/src/streaming_ingestion.py
from pyspark.sql import SparkSession
from pyspark.sql.functions import from_json, col, current_timestamp
from pyspark.sql.types import StructType, StructField, StringType, TimestampType
import logging

logger = logging.getLogger(__name__)

class StreamingIngestion:
    """Ingest streaming data from Kafka to Delta Lake"""

    def __init__(self, spark: SparkSession, kafka_brokers: str):
        self.spark = spark
        self.kafka_brokers = kafka_brokers

    def start_agent_event_stream(self, bronze_path: str):
        """Start streaming agent events to bronze layer"""

        # Define schema for agent events
        event_schema = StructType([
            StructField("eventId", StringType(), False),
            StructField("tenantId", StringType(), False),
            StructField("agentId", StringType(), False),
            StructField("timestamp", StringType(), False),
            StructField("eventType", StringType(), False),
            StructField("payload", StringType(), True)
        ])

        # Read from Kafka
        df = self.spark \
            .readStream \
            .format("kafka") \
            .option("kafka.bootstrap.servers", self.kafka_brokers) \
            .option("subscribe", "arcan.agents") \
            .option("startingOffsets", "latest") \
            .load()

        # Parse JSON
        parsed_df = df.select(
            from_json(col("value").cast("string"), event_schema).alias("data")
        ).select(
            "data.*",
            current_timestamp().alias("ingestion_timestamp")
        )

        # Write to Delta Lake bronze layer
        query = parsed_df \
            .writeStream \
            .format("delta") \
            .outputMode("append") \
            .option("checkpointLocation", f"{bronze_path}/_checkpoints/agent_events") \
            .trigger(processingTime="10 seconds") \
            .start(f"{bronze_path}/agent_events")

        logger.info("Started agent event streaming to bronze layer")
        return query
```

## Week 4: Integration and Testing

### Day 1-2: Integration Tests

1. **Schema Validation Tests**

```python
# packages/core-schemas/tests/test_schema_validation.py
import pytest
import json
from jsonschema import validate, ValidationError
from pathlib import Path

class TestSchemaValidation:
    """Test JSON schema validation"""

    @pytest.fixture
    def agent_event_schema(self):
        schema_path = Path(__file__).parent.parent / "json/event/agent-interaction.json"
        with open(schema_path) as f:
            return json.load(f)

    def test_valid_agent_event(self, agent_event_schema):
        """Test valid agent event"""
        valid_event = {
            "eventId": "123e4567-e89b-12d3-a456-426614174000",
            "tenantId": "123e4567-e89b-12d3-a456-426614174001",
            "agentId": "test_agent_1",
            "timestamp": "2024-01-01T00:00:00Z",
            "eventType": "message.received",
            "payload": {"message": "Hello"}
        }

        # Should not raise
        validate(instance=valid_event, schema=agent_event_schema)

    def test_invalid_agent_event_missing_field(self, agent_event_schema):
        """Test invalid agent event with missing required field"""
        invalid_event = {
            "eventId": "123e4567-e89b-12d3-a456-426614174000",
            "tenantId": "123e4567-e89b-12d3-a456-426614174001",
            # Missing agentId
            "timestamp": "2024-01-01T00:00:00Z",
            "eventType": "message.received",
            "payload": {"message": "Hello"}
        }

        with pytest.raises(ValidationError):
            validate(instance=invalid_event, schema=agent_event_schema)
```

2. **Event Streaming Integration Test**

```python
# packages/event-streaming/tests/test_integration.py
import pytest
import asyncio
from sqlalchemy.ext.asyncio import create_async_engine, AsyncSession
from sqlalchemy.orm import sessionmaker
from testcontainers.kafka import KafkaContainer

from packages.event_streaming.src.publisher import KafkaEventPublisher
from packages.event_streaming.src.outbox import OutboxEvent
from packages.event_streaming.src.outbox_publisher import OutboxPublisherService

@pytest.mark.asyncio
class TestEventStreamingIntegration:
    """Integration tests for event streaming"""

    @pytest.fixture
    async def kafka_container(self):
        """Start Kafka container for testing"""
        with KafkaContainer() as kafka:
            yield kafka

    @pytest.fixture
    async def db_session(self):
        """Create test database session"""
        engine = create_async_engine("sqlite+aiosqlite:///:memory:")
        async with engine.begin() as conn:
            await conn.run_sync(OutboxEvent.metadata.create_all)

        async_session = sessionmaker(
            engine, class_=AsyncSession, expire_on_commit=False
        )

        async with async_session() as session:
            yield session

    async def test_outbox_to_kafka_flow(self, kafka_container, db_session):
        """Test full flow from outbox to Kafka"""
        # Create outbox event
        event = OutboxEvent(
            aggregate_id="test_agent_1",
            event_type="agent.created.v1",
            topic="test.agents",
            payload={"name": "Test Agent", "type": "test"}
        )
        db_session.add(event)
        await db_session.commit()

        # Create publisher
        publisher = KafkaEventPublisher(
            bootstrap_servers=kafka_container.get_bootstrap_server()
        )

        # Create outbox service
        session_factory = lambda: db_session
        outbox_service = OutboxPublisherService(
            session_factory=session_factory,
            event_publisher=publisher
        )

        # Process one batch
        await publisher.start()
        await outbox_service._process_batch()
        await publisher.stop()

        # Verify event was published
        await db_session.refresh(event)
        assert event.status == "PUBLISHED"
        assert event.published_at is not None
```

### Day 3-4: Local Development Environment

1. **Docker Compose Setup**

```yaml
# docker-compose.yml
version: '3.8'

services:
  postgres:
    image: postgres:15-alpine
    environment:
      POSTGRES_USER: arcan
      POSTGRES_PASSWORD: arcan_dev
      POSTGRES_DB: arcan_dev
    ports:
      - '5432:5432'
    volumes:
      - postgres_data:/var/lib/postgresql/data

  redpanda:
    image: docker.redpanda.com/redpandadata/redpanda:latest
    command:
      - redpanda
      - start
      - --smp
      - '1'
      - --reserve-memory
      - 0M
      - --overprovisioned
      - --node-id
      - '0'
      - --kafka-addr
      - PLAINTEXT://0.0.0.0:29092,OUTSIDE://0.0.0.0:9092
      - --advertise-kafka-addr
      - PLAINTEXT://redpanda:29092,OUTSIDE://localhost:9092
    ports:
      - '9092:9092'
      - '9644:9644'
    volumes:
      - redpanda_data:/var/lib/redpanda/data

  minio:
    image: minio/minio:latest
    command: server /data --console-address ":9001"
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    ports:
      - '9000:9000'
      - '9001:9001'
    volumes:
      - minio_data:/data

  api:
    build:
      context: ./apps/api
      dockerfile: Dockerfile
    environment:
      DATABASE_URL: postgresql+asyncpg://arcan:arcan_dev@postgres/arcan_dev
      KAFKA_BOOTSTRAP_SERVERS: redpanda:29092
      S3_ENDPOINT_URL: http://minio:9000
      AWS_ACCESS_KEY_ID: minioadmin
      AWS_SECRET_ACCESS_KEY: minioadmin
    ports:
      - '8000:8000'
    depends_on:
      - postgres
      - redpanda
      - minio
    volumes:
      - ./apps/api:/app
      - ./packages:/packages

volumes:
  postgres_data:
  redpanda_data:
  minio_data:
```

2. **Local Development Script**

```bash
#!/bin/bash
# scripts/dev-setup.sh

echo "🚀 Setting up Arcan local development environment..."

# Check prerequisites
command -v docker >/dev/null 2>&1 || { echo "Docker is required but not installed. Aborting." >&2; exit 1; }
command -v bun >/dev/null 2>&1 || { echo "Bun is required but not installed. Aborting." >&2; exit 1; }
command -v uv >/dev/null 2>&1 || { echo "UV is required but not installed. Aborting." >&2; exit 1; }

# Install dependencies
echo "📦 Installing dependencies..."
bun install

# Build core schemas
echo "🔨 Building core schemas..."
cd packages/core-schemas && bun run build && cd ../..

# Start infrastructure
echo "🐳 Starting infrastructure services..."
docker-compose up -d postgres redpanda minio

# Wait for services
echo "⏳ Waiting for services to be ready..."
sleep 10

# Run database migrations
echo "🗄️ Running database migrations..."
cd apps/api && uv run alembic upgrade head && cd ../..

# Create Kafka topics
echo "📨 Creating Kafka topics..."
docker exec -it arcan_redpanda_1 rpk topic create arcan.agents arcan.workflows arcan.events

# Create MinIO buckets
echo "🪣 Creating MinIO buckets..."
docker exec -it arcan_minio_1 mc alias set local http://localhost:9000 minioadmin minioadmin
docker exec -it arcan_minio_1 mc mb local/arcan-lakehouse

echo "✅ Development environment ready!"
echo "   - API: http://localhost:8000"
echo "   - MinIO Console: http://localhost:9001"
echo "   - Redpanda Console: http://localhost:9644"
```

### Day 5: Documentation and CI/CD

1. **Update GitHub Actions Workflow**

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main]

jobs:
  build-and-test:
    runs-on: ubuntu-latest

    strategy:
      matrix:
        node-version: [18.x, 20.x]
        python-version: ['3.10', '3.11', '3.12']

    steps:
      - uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: ${{ matrix.node-version }}

      - name: Setup Python
        uses: actions/setup-python@v4
        with:
          python-version: ${{ matrix.python-version }}

      - name: Install Bun
        uses: oven-sh/setup-bun@v1

      - name: Install UV
        run: curl -LsSf https://astral.sh/uv/install.sh | sh

      - name: Cache Turborepo
        uses: actions/cache@v3
        with:
          path: .turbo
          key: ${{ runner.os }}-turbo-${{ github.sha }}
          restore-keys: |
            ${{ runner.os }}-turbo-

      - name: Install dependencies
        run: bun install

      - name: Build packages
        run: bun run build

      - name: Run linting
        run: bun run lint

      - name: Run tests
        run: bun run test

      - name: Upload coverage
        uses: codecov/codecov-action@v3
        with:
          files: ./coverage.xml,./apps/api/coverage.xml
```

2. **Developer Documentation**

````markdown
# packages/core-schemas/README.md

# Arcan Core Schemas

This package contains the canonical JSON Schema definitions for all data structures in the Arcan platform.

## Structure

- `json/` - JSON Schema definitions
  - `common/` - Common types and definitions
  - `agent/` - Agent-related schemas
  - `event/` - Event schemas
  - `workflow/` - Workflow schemas
  - `finance/` - Financial/payment schemas
- `python/` - Generated Pydantic models
- `typescript/` - Generated TypeScript interfaces

## Usage

### In Python Services

```python
from arcan.core_schemas.python.models import AgentInteractionEvent

event = AgentInteractionEvent(
    tenant_id="...",
    agent_id="...",
    event_type="message.received",
    payload={"message": "Hello"}
)
```
````

### In TypeScript Applications

```typescript
import { AgentInteractionEvent } from '@arcan/core-schemas';

const event: AgentInteractionEvent = {
  tenantId: '...',
  agentId: '...',
  eventType: 'message.received',
  payload: { message: 'Hello' },
};
```

## Development

To add a new schema:

1. Create the JSON Schema file in the appropriate directory
2. Run `bun run build` to generate code
3. Commit all changes (including generated code)

## Versioning

Schemas follow semantic versioning. Breaking changes require:

1. New version file (e.g., `v2_0_0_AgentEvent.json`)
2. Migration guide in `MIGRATIONS.md`
3. DAO approval for shared schemas

```

## Summary

This roadmap provides a concrete implementation plan for Phase 1 of Arcan development, focusing on:

1. **Schema Management**: Establishing the foundation for consistent data structures across the platform
2. **Event-Driven Architecture**: Implementing reliable event streaming with Kafka/Redpanda
3. **Data Platform**: Setting up Delta Lake with proper governance
4. **Integration**: Ensuring all components work together seamlessly

Each week builds upon the previous, creating a solid foundation for the agent framework and advanced features in subsequent phases.
```
