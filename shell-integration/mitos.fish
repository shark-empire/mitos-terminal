# MITOS Terminal — shell integration for fish (OSC 7 + OSC 133).
# Add to ~/.config/fish/config.fish:
#   test -n "$MITOS_SHELL_INTEGRATION_DIR"; and source "$MITOS_SHELL_INTEGRATION_DIR/mitos.fish"

status is-interactive; or return 0
test "$TERM_PROGRAM" = "mitos-terminal"; or return 0
set -q MITOS_SHELL_INTEGRATION_LOADED; and return 0
set -g MITOS_SHELL_INTEGRATION_LOADED 1

function __mitos_report_cwd --on-event fish_prompt
    printf '\e]7;file://%s%s\a' (hostname) (string escape --style=url -- $PWD)
end

function __mitos_preexec --on-event fish_preexec
    set -g __mitos_running 1
    printf '\e]133;C\a'
end

function __mitos_postexec --on-event fish_postexec
    set -l last_status $status
    if set -q __mitos_running
        printf '\e]133;D;%s\a' $last_status
        set -e __mitos_running
    end
end

# A/B marks: wrap the user's prompt function.
if functions -q fish_prompt
    functions -c fish_prompt __mitos_original_fish_prompt
    function fish_prompt
        printf '\e]133;A\a'
        __mitos_original_fish_prompt
        printf '\e]133;B\a'
    end
end
