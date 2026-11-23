---
{
  "id": "finding/pair.gap",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/pair.gap": "e999678d047a71c30e12f6076a4e075cae239faaa151e58c0755d98e6e24ddec"
  },
  "cause": "The measured edge-to-edge spacing of the paired tracks differs from their assigned gap beyond the allowed tolerance.",
  "fix": "Reroute the pair at its gap",
  "fixed_by": [
    "entry/layout.route_pair"
  ],
  "rule_key": "pair_gap_tolerance",
  "explanation": "The pair gap is resolved per layer from the impedance target or class geometry, with track-to-track clearance as a lower bound. The class can set a gap tolerance. Otherwise the checker takes the greater of the profile’s absolute tolerance and its share of the gap."
}
---
