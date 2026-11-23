---
{
  "id": "finding/rule.budget",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/rule.budget": "75b3c70f3bdd6cc6dde968cc0d1679857e721882044a9cbb826bbf1734bdca3b"
  },
  "cause": "A pin rule exhausted its evaluation budget before it could check the assignment.",
  "fix": "Simplify the rule’s computation and run its samples again",
  "fixed_by": [
    "entry/library.edit"
  ],
  "title": "Pin rule reached its evaluation limit"
}
---
