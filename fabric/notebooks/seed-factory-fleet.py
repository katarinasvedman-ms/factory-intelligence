# Fabric notebook source

# METADATA ********************

# META {
# META   "kernel_info": {
# META     "name": "synapse_pyspark"
# META   },
# META   "dependencies": {}
# META }

# CELL ********************

# This notebook appends a small, rerunnable seven-day fleet history directly to
# the Governed Floor Eventhouse table. Stable event IDs let analytical queries keep
# only the latest seeded copy after repeated runs.

from datetime import datetime, timedelta, timezone
import json

from pyspark.sql import functions as F
from pyspark.sql.types import (
    LongType,
    MapType,
    StringType,
    StructField,
    StructType,
    TimestampType,
)

KUSTO_URI = "__KUSTO_URI__"
KUSTO_DATABASE = "__KUSTO_DATABASE__"
KUSTO_TABLE = "factory_incident_events"
ANCHOR_UTC = ""
PUBLISH = True

# METADATA ********************

# META {
# META   "language": "python",
# META   "language_group": "synapse_pyspark",
# META   "tags": [
# META     "parameters"
# META   ]
# META }

# CELL ********************

spark.conf.set("spark.sql.session.timeZone", "UTC")

if ANCHOR_UTC:
    anchor = datetime.fromisoformat(ANCHOR_UTC.replace("Z", "+00:00"))
else:
    anchor = datetime.now(timezone.utc)
anchor = anchor.astimezone(timezone.utc).replace(microsecond=0)

events = []


def add(
    factory_id,
    line_id,
    machine_id,
    incident_id,
    event_key,
    hours_before_anchor,
    event_type,
    incident_status,
    severity,
    correlation_id=None,
    details=None,
):
    events.append(
        {
            "schema_version": 1,
            "event_id": f"fleet-week-v1:{factory_id}:{incident_id}:{event_key}",
            "occurred_at": (anchor - timedelta(hours=hours_before_anchor)).replace(
                tzinfo=None
            ),
            "factory_id": factory_id,
            "line_id": line_id,
            "machine_id": machine_id,
            "incident_id": incident_id,
            "event_type": event_type,
            "incident_status": incident_status,
            "severity": severity,
            "summary": f"Synthetic governed lifecycle event: {event_type}.",
            "correlation_id": correlation_id,
            "details": details or {},
        }
    )


rv_coolant = "fleet-seed-riverton-coolant-01"
rv_robot = "fleet-seed-riverton-robot-01"
rv_bearing = "fleet-seed-riverton-bearing-01"
ls_bearing_history = "fleet-seed-lakeside-bearing-01"
ls_bearing_current = "fleet-seed-lakeside-bearing-02"
ls_packer = "fleet-seed-lakeside-packer-01"
ls_oven = "fleet-seed-lakeside-oven-01"

add("factory-riverton-02", "Line C", "Coolant Pump 07", rv_coolant, "detected", 150, "coolant_pressure_low_detected", "detected", "high")
add("factory-riverton-02", "Line C", "Coolant Pump 07", rv_coolant, "assessed", 149, "local_assessment_completed", "locally_assessed", "high")
add("factory-riverton-02", "Line C", "Weld Robot 08", rv_robot, "detected", 149, "downstream_temperature_alarm_detected", "detected", "medium", rv_coolant)
add("factory-riverton-02", "Line C", "Coolant Pump 07", rv_coolant, "advisory", 148, "factory_advisory_available", "awaiting_approval", "high")
add("factory-riverton-02", "Line C", "Weld Robot 08", rv_robot, "correlated", 148, "upstream_correlation_identified", "monitoring", "medium", rv_coolant)
add("factory-riverton-02", "Line C", "Coolant Pump 07", rv_coolant, "executed", 147, "guard_executed", "executed", "high", details={"requested_reduction_percent": "10", "executed": "true"})
add("factory-riverton-02", "Line C", "Weld Robot 08", rv_robot, "upstream-action", 147, "upstream_action_executed", "monitoring", "medium", rv_coolant)
add("factory-riverton-02", "Line C", "Coolant Pump 07", rv_coolant, "resolved", 144, "incident_resolved", "resolved", "high")
add("factory-riverton-02", "Line C", "Weld Robot 08", rv_robot, "resolved", 143, "incident_resolved", "resolved", "medium", rv_coolant)

add("factory-riverton-02", "Line A", "Drive Motor 22", rv_bearing, "detected", 100, "bearing_vibration_detected", "detected", "high")
add("factory-riverton-02", "Line A", "Drive Motor 22", rv_bearing, "advisory", 98, "factory_advisory_available", "awaiting_approval", "high")
add("factory-riverton-02", "Line A", "Drive Motor 22", rv_bearing, "executed", 97, "guard_executed", "executed", "high", details={"requested_reduction_percent": "12", "executed": "true"})
add("factory-riverton-02", "Line A", "Drive Motor 22", rv_bearing, "resolved", 94, "incident_resolved", "resolved", "high")

add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_history, "detected", 124, "bearing_vibration_detected", "detected", "high")
add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_history, "advisory", 122, "factory_advisory_available", "awaiting_approval", "high")
add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_history, "executed", 121, "guard_executed", "executed", "high", details={"requested_reduction_percent": "10", "executed": "true"})
add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_history, "resolved", 118, "incident_resolved", "resolved", "high")

add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_current, "detected", 26, "bearing_vibration_detected", "detected", "high")
add("factory-lakeside-03", "Line B", "Packer 09", ls_packer, "detected", 25, "downstream_starvation_detected", "detected", "medium", ls_bearing_current)
add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_current, "advisory", 24, "factory_advisory_available", "awaiting_approval", "high")
add("factory-lakeside-03", "Line B", "Packer 09", ls_packer, "correlated", 23, "upstream_correlation_identified", "monitoring", "medium", ls_bearing_current)
add("factory-lakeside-03", "Line B", "Drive Motor 14", ls_bearing_current, "monitoring", 2, "inspection_monitoring_started", "monitoring", "high")
add("factory-lakeside-03", "Line B", "Packer 09", ls_packer, "monitoring", 1, "downstream_monitoring_continues", "monitoring", "medium", ls_bearing_current)

add("factory-lakeside-03", "Line D", "Cure Oven 03", ls_oven, "detected", 72, "oven_temperature_deviation_detected", "detected", "critical")
add("factory-lakeside-03", "Line D", "Cure Oven 03", ls_oven, "advisory", 71, "factory_advisory_available", "awaiting_approval", "critical")
add("factory-lakeside-03", "Line D", "Cure Oven 03", ls_oven, "rejected", 70, "guard_rejected", "rejected", "critical", details={"requested_reduction_percent": "40", "executed": "false"})
add("factory-lakeside-03", "Line D", "Cure Oven 03", ls_oven, "resolved", 66, "incident_resolved", "resolved", "critical")

schema = StructType(
    [
        StructField("schema_version", LongType(), False),
        StructField("event_id", StringType(), False),
        StructField("occurred_at", TimestampType(), False),
        StructField("factory_id", StringType(), False),
        StructField("line_id", StringType(), False),
        StructField("machine_id", StringType(), False),
        StructField("incident_id", StringType(), False),
        StructField("event_type", StringType(), False),
        StructField("incident_status", StringType(), False),
        StructField("severity", StringType(), False),
        StructField("summary", StringType(), False),
        StructField("correlation_id", StringType(), True),
        StructField(
            "details", MapType(StringType(), StringType(), True), False
        ),
    ]
)

seed_df = spark.createDataFrame(events, schema)

assert seed_df.count() == 27
assert seed_df.select("factory_id").distinct().count() == 2
assert seed_df.select("incident_id").distinct().count() == 7

display(
    seed_df.groupBy("factory_id")
    .agg(
        F.count("*").alias("event_rows"),
        F.countDistinct("incident_id").alias("distinct_incidents"),
        F.min("occurred_at").alias("represented_start"),
        F.max("occurred_at").alias("represented_end"),
    )
    .orderBy("factory_id")
)

# METADATA ********************

# META {
# META   "language": "python",
# META   "language_group": "synapse_pyspark"
# META }

# CELL ********************

if PUBLISH:
    access_token = mssparkutils.credentials.getToken("kusto")
    (
        seed_df.write.format("com.microsoft.kusto.spark.synapse.datasource")
        .option("kustoCluster", KUSTO_URI)
        .option("kustoDatabase", KUSTO_DATABASE)
        .option("kustoTable", KUSTO_TABLE)
        .option("accessToken", access_token)
        .mode("Append")
        .save()
    )

    verification_query = f"""
    let events =
        {KUSTO_TABLE}
        | summarize arg_max(occurred_at, *) by event_id;
    events
    | where event_id startswith "fleet-week-v1:"
    | summarize
        event_rows=count(),
        distinct_incidents=dcount(incident_id),
        represented_start=min(occurred_at),
        represented_end=max(occurred_at)
      by factory_id
    | order by factory_id asc
    """
    verification_df = (
        spark.read.format("com.microsoft.kusto.spark.synapse.datasource")
        .option("accessToken", access_token)
        .option("kustoCluster", KUSTO_URI)
        .option("kustoDatabase", KUSTO_DATABASE)
        .option("kustoQuery", verification_query)
        .load()
    )
    display(verification_df)
    result = {
        "published": True,
        "anchor_utc": anchor.isoformat(),
        "event_rows": 27,
        "distinct_incidents": 7,
        "factories": ["factory-lakeside-03", "factory-riverton-02"],
    }
else:
    result = {
        "published": False,
        "anchor_utc": anchor.isoformat(),
        "event_rows": 27,
        "distinct_incidents": 7,
        "factories": ["factory-lakeside-03", "factory-riverton-02"],
    }

mssparkutils.notebook.exit(json.dumps(result))

# METADATA ********************

# META {
# META   "language": "python",
# META   "language_group": "synapse_pyspark"
# META }
