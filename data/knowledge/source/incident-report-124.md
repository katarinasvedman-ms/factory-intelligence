# Incident Report 124

Document ID: INCIDENT-124  
Date: 2026-04-18  
Classification: Synthetic demo data

Three machines on Line A reported vibration warnings during a high-load production
window. Initial review focused on individual bearings. Peer-machine correlation
later identified a shared alignment and load-distribution condition.

The corrective workflow was:

1. reduce line speed under operator control;
2. verify sensor readings;
3. inspect alignment and mounting points;
4. compare motor-current trends across the line;
5. restore speed incrementally after verification.

Lesson: when more than one machine on a line reports related vibration behavior,
factory-level correlation should occur before declaring an individual component
failure.
