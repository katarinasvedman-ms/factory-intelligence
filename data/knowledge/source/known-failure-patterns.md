# Known Failure Patterns

Document ID: FAIL-PATTERN-022  
Revision: 4  
Classification: Synthetic demo data

## Pattern A: Single-machine bearing degradation

- one machine affected;
- vibration and temperature rise together;
- peer machines remain normal;
- maintenance history shows increasing vibration over time.

## Pattern B: Shared line load condition

- multiple machines on the same line report vibration or load warnings;
- motor current changes correlate with production load;
- individual bearing replacement does not explain all events.

## Pattern C: Sensor or mounting issue

- primary sensor differs from an independent measurement;
- abrupt value change occurs without a corresponding current or temperature trend;
- recent sensor or mounting work appears in maintenance history.

Slow-path analysis should compare the current evidence with all three patterns and
state what additional evidence would distinguish them.
