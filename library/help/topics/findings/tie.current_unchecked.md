---
{
  "id": "finding/tie.current_unchecked",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.current_unchecked": "cc7581302a9d9f16864e0c920f3ab828eed97accb7ec88139daa9019d9509ec1"
  },
  "cause": "The tie has a required current, but no temperature rise and current table to rate its bridge against.",
  "fix": "Set the temperature rise and supply a sourced current table",
  "fixed_by": [
    "entry/rules.set_tie_current"
  ]
}
---
