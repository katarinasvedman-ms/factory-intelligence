# Robot 17 Service Manual

Document ID: ROBOT17-SVC-001  
Revision: 3  
Classification: Synthetic demo data

## Bearing alarm response

Alarm `BRG-VIB-CRIT` indicates that vibration and bearing-temperature thresholds
were exceeded together. Treat the alarm as evidence of an abnormal condition, not
as a confirmed root cause.

Before returning the robot to normal production speed:

1. Verify the vibration and temperature sensors against an independent reading.
2. Inspect the bearing housing, mounting points, guards, and lubrication condition.
3. Review peer-machine events for a line-wide load or alignment condition.
4. Review maintenance history for recent bearing, lubrication, or alignment work.
5. Keep the robot paused or at an operator-approved reduced speed until inspection
   criteria are satisfied.

Do not bypass safety interlocks or use an AI-generated response as authorization to
resume operation.
