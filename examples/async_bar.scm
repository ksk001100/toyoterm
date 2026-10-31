;; Slow commands run outside both the script and GUI threads.
(bind-action! "CTRL+SHIFT+T" 'new-tab)
(bind-action! "CTRL+SHIFT+R" 'reload-config)
(bar! 'bottom 'center " | "
  (list
    (async-widget "ping" (list (if (equal? (toyoterm-platform) 'windows) "-n" "-c") "1" "1.1.1.1")
      10.0 "Ping: measuring..." (lambda (result) (if (process-success? result) "Ping: ok" "Ping: failed")))
    (async-widget "curl" '("-s" "https://wttr.in/?format=1")
      300.0 "Weather: fetching..." (lambda (result) (if (process-success? result) (process-stdout-text result) "Weather: unavailable")))
    (widget (lambda (ctx) (string-append "Host: " (symbol->string (toyoterm-platform)))) 1.0)))

;; Resolve the active pane's repository for each asynchronous refresh.
(bar! 'bottom 'right " | "
  (list
    (async-widget "git" '("branch" "--show-current") 2.0 ""
      (lambda (result)
        (if (process-success? result) (trim (process-stdout-text result)) ""))
      (lambda (ctx) (handle-ref (context-ref ctx 'pane) 'cwd)))))
