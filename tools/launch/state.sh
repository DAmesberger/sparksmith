# Allocate state for one app process. Keep the path with its capture metadata.
capture_state() {
  local parent="$1" metadata
  mkdir -p "$parent"
  state="$(mktemp -d "$parent/state.XXXXXXXX")"
  metadata="${2:-$state}"
  mkdir "$state/recovery" "$state/config" "$state/cache" "$state/home"
  export SPARKSMITH_RECOVERY_DIR="$state/recovery"
  export SPARKSMITH_CONFIG_DIR="$state/config"
  export SPARKSMITH_CACHE_DIR="$state/cache"
  printf '%s\n' "$state" > "$metadata/state.path"
}
