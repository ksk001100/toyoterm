#include <stdlib.h>
#include <stdint.h>
#include <string.h>

#include "mruby.h"

extern int toyoterm_host_read_file(const uint8_t *path, size_t path_length,
                                   uint8_t **output, size_t *output_length,
                                   char **error);
extern int toyoterm_host_spawn(const uint8_t *const *arguments,
                               const size_t *lengths, size_t count,
                               const uint8_t *cwd, size_t cwd_length,
                               int cwd_available,
                               uint8_t **stdout_output, size_t *stdout_length,
                               uint8_t **stderr_output, size_t *stderr_length,
                               int32_t *exit_status, char **error);
extern void toyoterm_host_bytes_free(uint8_t *bytes, size_t length);
extern void toyoterm_host_string_free(char *string);

void *toyoterm_mruby_open(void) {
  mrb_state *mrb = mrb_open();
  if (mrb == NULL) {
    return NULL;
  }
  if (mrb->exc != NULL) {
    mrb_close(mrb);
    return NULL;
  }
  return mrb;
}

void toyoterm_mruby_close(void *state) {
  if (state != NULL) {
    mrb_close((mrb_state *)state);
  }
}

typedef struct {
  mrb_ccontext *compiler;
  mrb_value binding;
} toyoterm_console_context;

void *toyoterm_mruby_console_context_new(void *state) {
  mrb_state *mrb = (mrb_state *)state;
  toyoterm_console_context *context = malloc(sizeof(*context));
  if (context == NULL) {
    return NULL;
  }
  context->compiler = mrb_ccontext_new(mrb);
  if (context->compiler == NULL) {
    free(context);
    return NULL;
  }
  context->compiler->capture_errors = TRUE;
  context->compiler->lineno = 1;
  mrb_ccontext_filename(mrb, context->compiler, "(toyoterm ruby console)");
  context->binding = mrb_load_string(mrb, "proc { }.binding");
  if (mrb->exc != NULL) {
    mrb->exc = NULL;
    mrb_ccontext_free(mrb, context->compiler);
    free(context);
    return NULL;
  }
  mrb_gc_register(mrb, context->binding);
  return context;
}

void toyoterm_mruby_console_context_free(void *state, void *context) {
  if (state != NULL && context != NULL) {
    toyoterm_console_context *console = (toyoterm_console_context *)context;
    mrb_gc_unregister((mrb_state *)state, console->binding);
    mrb_ccontext_free((mrb_state *)state, console->compiler);
    free(console);
  }
}

static char *copy_mruby_string(mrb_value value) {
  mrb_int length = RSTRING_LEN(value);
  char *copy = malloc((size_t)length + 1);
  if (copy == NULL) {
    return NULL;
  }
  memcpy(copy, RSTRING_PTR(value), (size_t)length);
  copy[length] = '\0';
  return copy;
}

static mrb_value format_exception(mrb_state *mrb, mrb_value exception) {
  mrb_value error = mrb_inspect(mrb, exception);
  if (mrb->exc != NULL) {
    mrb->exc = NULL;
    return error;
  }

  mrb_value backtrace = mrb_funcall(mrb, exception, "backtrace", 0);
  if (mrb->exc != NULL || !mrb_array_p(backtrace) || RARRAY_LEN(backtrace) == 0) {
    mrb->exc = NULL;
    return error;
  }

  mrb_value joined = mrb_ary_join(mrb, backtrace, mrb_str_new_lit(mrb, "\n"));
  if (mrb->exc != NULL) {
    mrb->exc = NULL;
    return error;
  }
  mrb_value message = mrb_str_dup(mrb, error);
  mrb_str_cat_lit(mrb, message, "\n");
  mrb_str_cat_str(mrb, message, joined);
  return message;
}

/* This follows mruby-bin-mirb's parser-based multiline classification. */
static mrb_bool console_code_block_open(struct mrb_parser_state *parser) {
  if (parser->parsing_heredoc != NULL || parser->lex_strterm != NULL) {
    return TRUE;
  }
  if (parser->nerr > 0) {
    static const char unexpected_end[] =
        "syntax error, unexpected end of file";
    static const char unexpected_eoi[] =
        "syntax error, unexpected end-of-input";
    const char *message = parser->error_buffer[0].message;
    return message != NULL &&
           (strncmp(message, unexpected_end, sizeof(unexpected_end) - 1) == 0 ||
            strncmp(message, unexpected_eoi, sizeof(unexpected_eoi) - 1) == 0);
  }
  switch (parser->lstate) {
  case EXPR_DOT:
  case EXPR_CLASS:
  case EXPR_FNAME:
  case EXPR_VALUE:
    return TRUE;
  default:
    return FALSE;
  }
}

int toyoterm_mruby_console_eval(void *state, void *console_context,
                                const char *source, char **output) {
  mrb_state *mrb = (mrb_state *)state;
  toyoterm_console_context *console =
      (toyoterm_console_context *)console_context;
  mrb_ccontext *context = console->compiler;
  int arena_index = mrb_gc_arena_save(mrb);
  *output = NULL;
  mrb->exc = NULL;

  struct mrb_parser_state *parser = mrb_parser_new(mrb);
  if (parser == NULL) {
    mrb_gc_arena_restore(mrb, arena_index);
    return 2;
  }
  parser->s = source;
  parser->send = source + strlen(source);
  parser->lineno = context->lineno;
  mrb_parser_parse(parser, context);
  if (console_code_block_open(parser)) {
    static const char incomplete[] = "syntax error, unexpected end of file";
    mrb_parser_free(parser);
    *output = malloc(sizeof(incomplete));
    if (*output != NULL) {
      memcpy(*output, incomplete, sizeof(incomplete));
    }
    mrb_gc_arena_restore(mrb, arena_index);
    return *output == NULL ? 2 : 3;
  }

  mrb_value value;
  if (parser->tree == NULL || parser->nerr > 0) {
    value = mrb_load_exec(mrb, parser, context);
  } else {
    mrb_parser_free(parser);
    mrb_value arguments[] = {
        mrb_str_new_cstr(mrb, source),
        mrb_str_new_lit(mrb, "(toyoterm ruby console)"),
        mrb_int_value(mrb, 1),
    };
    value = mrb_funcall_argv(mrb, console->binding, mrb_intern_lit(mrb, "eval"),
                             3, arguments);
  }
  for (const char *cursor = source; *cursor != '\0'; cursor++) {
    if (*cursor == '\n') {
      context->lineno++;
    }
  }
  context->lineno++;
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb_gc_protect(mrb, exception);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    mrb_gc_arena_restore(mrb, arena_index);
    return *output == NULL ? 2 : 1;
  }

  value = mrb_funcall_argv(mrb, value, mrb_intern_lit(mrb, "inspect"), 0, NULL);
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb_gc_protect(mrb, exception);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    mrb_gc_arena_restore(mrb, arena_index);
    return *output == NULL ? 2 : 1;
  }
  if (!mrb_string_p(value)) {
    value = mrb_obj_as_string(mrb, value);
  }
  *output = copy_mruby_string(value);
  int status = *output == NULL ? 2 : 0;
  mrb_gc_arena_restore(mrb, arena_index);
  return status;
}

static int finish_typed_call(mrb_state *mrb, char **error_output) {
  if (mrb->exc == NULL) {
    return 0;
  }
  mrb_value exception = mrb_obj_value(mrb->exc);
  mrb_gc_protect(mrb, exception);
  mrb->exc = NULL;
  mrb_value error = format_exception(mrb, exception);
  *error_output = copy_mruby_string(error);
  mrb->exc = NULL;
  return *error_output == NULL ? 2 : 1;
}

/*
 * Rust calls the functions below as top-level VM entry points.  mruby's GC
 * arena is a stack of temporary roots, so every entry point must restore the
 * stack after it has copied any result or exception into C-owned memory.
 * Objects installed into Ruby instance/module variables remain reachable via
 * Ruby's object graph and do not need to remain in the arena.
 */
static int finish_arena_call(mrb_state *mrb, int arena_index,
                             char **error_output) {
  int status = finish_typed_call(mrb, error_output);
  mrb_gc_arena_restore(mrb, arena_index);
  return status;
}

static mrb_value toyoterm_module(mrb_state *mrb) {
  return mrb_obj_value(mrb_module_get(mrb, "Toyoterm"));
}

static mrb_value integer_array(mrb_state *mrb, const uint64_t *values,
                               size_t length) {
  mrb_value array = mrb_ary_new_capa(mrb, (mrb_int)length);
  for (size_t index = 0; index < length; index++) {
    mrb_ary_push(mrb, array, mrb_int_value(mrb, (mrb_int)values[index]));
  }
  return array;
}

static mrb_value host_read_file(mrb_state *mrb, mrb_value self) {
  (void)self;
  mrb_value path;
  mrb_get_args(mrb, "S", &path);
  uint8_t *output = NULL;
  size_t output_length = 0;
  char *error = NULL;
  int status = toyoterm_host_read_file(
      (const uint8_t *)RSTRING_PTR(path), (size_t)RSTRING_LEN(path), &output,
      &output_length, &error);
  if (status != 0) {
    mrb_value message = mrb_str_new_cstr(mrb, error == NULL ? "read file failed" : error);
    toyoterm_host_string_free(error);
    mrb_exc_raise(mrb, mrb_exc_new_str(mrb, E_RUNTIME_ERROR, message));
  }
  mrb_value result =
      mrb_str_new(mrb, output == NULL ? "" : (const char *)output,
                  (mrb_int)output_length);
  toyoterm_host_bytes_free(output, output_length);
  return result;
}

static mrb_value host_spawn(mrb_state *mrb, mrb_value self) {
  (void)self;
  mrb_value arguments;
  mrb_value cwd;
  mrb_get_args(mrb, "Ao", &arguments, &cwd);
  mrb_int count = RARRAY_LEN(arguments);
  const uint8_t **pointers = calloc((size_t)count, sizeof(*pointers));
  size_t *lengths = calloc((size_t)count, sizeof(*lengths));
  if ((count > 0) && (pointers == NULL || lengths == NULL)) {
    free(pointers);
    free(lengths);
    mrb_raise(mrb, E_RUNTIME_ERROR, "allocate process arguments failed");
  }
  for (mrb_int index = 0; index < count; index++) {
    mrb_value argument = mrb_ary_ref(mrb, arguments, index);
    if (!mrb_string_p(argument)) {
      free(pointers);
      free(lengths);
      mrb_raise(mrb, E_TYPE_ERROR, "process arguments must be strings");
    }
    pointers[index] = (const uint8_t *)RSTRING_PTR(argument);
    lengths[index] = (size_t)RSTRING_LEN(argument);
  }

  uint8_t *stdout_output = NULL;
  uint8_t *stderr_output = NULL;
  size_t stdout_length = 0;
  size_t stderr_length = 0;
  int32_t exit_status = -1;
  char *error = NULL;
  const uint8_t *cwd_pointer = NULL;
  size_t cwd_length = 0;
  int cwd_available = !mrb_nil_p(cwd);
  if (cwd_available) {
    if (!mrb_string_p(cwd)) {
      free(pointers);
      free(lengths);
      mrb_raise(mrb, E_TYPE_ERROR, "process cwd must be a string or nil");
    }
    cwd_pointer = (const uint8_t *)RSTRING_PTR(cwd);
    cwd_length = (size_t)RSTRING_LEN(cwd);
  }
  int status = toyoterm_host_spawn(
      pointers, lengths, (size_t)count, cwd_pointer, cwd_length, cwd_available,
      &stdout_output, &stdout_length,
      &stderr_output, &stderr_length, &exit_status, &error);
  free(pointers);
  free(lengths);
  if (status != 0) {
    mrb_value message = mrb_str_new_cstr(mrb, error == NULL ? "spawn failed" : error);
    toyoterm_host_string_free(error);
    mrb_exc_raise(mrb, mrb_exc_new_str(mrb, E_RUNTIME_ERROR, message));
  }

  mrb_value result = mrb_ary_new_capa(mrb, 3);
  mrb_ary_push(mrb, result,
               mrb_str_new(mrb, stdout_output == NULL ? "" : (const char *)stdout_output,
                           (mrb_int)stdout_length));
  mrb_ary_push(mrb, result,
               mrb_str_new(mrb, stderr_output == NULL ? "" : (const char *)stderr_output,
                           (mrb_int)stderr_length));
  mrb_ary_push(mrb, result, mrb_int_value(mrb, (mrb_int)exit_status));
  toyoterm_host_bytes_free(stdout_output, stdout_length);
  toyoterm_host_bytes_free(stderr_output, stderr_length);
  return result;
}

void toyoterm_mruby_install_host_api(void *state) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  struct RClass *module = mrb_module_get(mrb, "Toyoterm");
  mrb_define_module_function(mrb, module, "__host_read_file", host_read_file,
                             MRB_ARGS_REQ(1));
  mrb_define_module_function(mrb, module, "__host_spawn", host_spawn,
                             MRB_ARGS_REQ(2));
  mrb_gc_arena_restore(mrb, arena_index);
}

int toyoterm_mruby_set_environment(void *state, const char *const *keys,
                                   const char *const *values,
                                   const size_t *lengths, size_t count,
                                   char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value entries = mrb_ary_new_capa(mrb, (mrb_int)(count * 2));
  for (size_t index = 0; index < count; index++) {
    mrb_ary_push(mrb, entries,
                 mrb_str_new(mrb, keys[index], (mrb_int)lengths[index * 2]));
    mrb_ary_push(mrb, entries,
                 mrb_str_new(mrb, values[index],
                             (mrb_int)lengths[index * 2 + 1]));
  }
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__replace_env"), 1, &entries);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_set_current_pane(void *state, uint64_t pane_id,
                                    char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value argument = mrb_int_value(mrb, (mrb_int)pane_id);
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__set_current_pane"), 1, &argument);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_set_live_handles(
    void *state, const uint64_t *workspaces, size_t workspace_count,
    const uint64_t *windows, size_t window_count, const uint64_t *tabs,
    size_t tab_count, const uint64_t *panes, size_t pane_count,
    char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[4] = {
      integer_array(mrb, workspaces, workspace_count),
      integer_array(mrb, windows, window_count),
      integer_array(mrb, tabs, tab_count),
      integer_array(mrb, panes, pane_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__replace_live_handles"), 4,
                   arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_reset_object_model(void *state, uint64_t workspace_id,
                                      uint64_t window_id, uint64_t tab_id,
                                      uint64_t pane_id, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[4] = {
      mrb_int_value(mrb, (mrb_int)workspace_id),
      mrb_int_value(mrb, (mrb_int)window_id),
      mrb_int_value(mrb, (mrb_int)tab_id),
      mrb_int_value(mrb, (mrb_int)pane_id),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__reset_object_model"), 4, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_add_workspace(void *state, uint64_t workspace_id,
                                 const char *name, size_t name_length,
                                 const uint64_t *windows, size_t window_count,
                                 char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[3] = {
      mrb_int_value(mrb, (mrb_int)workspace_id),
      mrb_str_new(mrb, name, (mrb_int)name_length),
      integer_array(mrb, windows, window_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_workspace"), 3, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_add_window(void *state, uint64_t window_id,
                              const uint64_t *tabs, size_t tab_count,
                              char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[2] = {
      mrb_int_value(mrb, (mrb_int)window_id),
      integer_array(mrb, tabs, tab_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_window"), 2, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_add_tab(void *state, uint64_t tab_id, const char *title,
                           size_t title_length, const uint64_t *panes,
                           size_t pane_count, int zoomed,
                           char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[4] = {
      mrb_int_value(mrb, (mrb_int)tab_id),
      mrb_str_new(mrb, title, (mrb_int)title_length),
      integer_array(mrb, panes, pane_count),
      mrb_bool_value(zoomed != 0),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_tab"), 4, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_add_pane(void *state, uint64_t pane_id, const char *title,
                            size_t title_length, const char *cwd,
                            size_t cwd_length, int cwd_available,
                            const char *remote_host,
                            size_t remote_host_length,
                            int remote_host_available,
                            uint64_t shell_integration_version,
                            int shell_integration_version_available,
                            const char *shell_integration_shell,
                            size_t shell_integration_shell_length,
                            int shell_integration_shell_available,
                            const char *const *user_var_keys,
                            const char *const *user_var_values,
                            const size_t *user_var_lengths,
                            size_t user_var_count, uint64_t pid,
                            int pid_available,
                            int command_running,
                            int32_t last_exit_status,
                            int last_exit_status_available,
                            const char *screen_text, size_t screen_text_length,
                            int zoomed, const char *icon_title,
                            size_t icon_title_length,
                            int icon_title_available, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value user_var_entries =
      mrb_ary_new_capa(mrb, (mrb_int)(user_var_count * 2));
  for (size_t index = 0; index < user_var_count; index++) {
    mrb_ary_push(
        mrb, user_var_entries,
        mrb_str_new(mrb, user_var_keys[index],
                    (mrb_int)user_var_lengths[index * 2]));
    mrb_ary_push(
        mrb, user_var_entries,
        mrb_str_new(mrb, user_var_values[index],
                    (mrb_int)user_var_lengths[index * 2 + 1]));
  }
  mrb_value arguments[13] = {
      mrb_int_value(mrb, (mrb_int)pane_id),
      mrb_str_new(mrb, title, (mrb_int)title_length),
      cwd_available ? mrb_str_new(mrb, cwd, (mrb_int)cwd_length)
                    : mrb_nil_value(),
      remote_host_available
          ? mrb_str_new(mrb, remote_host, (mrb_int)remote_host_length)
          : mrb_nil_value(),
      shell_integration_version_available
          ? mrb_int_value(mrb, (mrb_int)shell_integration_version)
          : mrb_nil_value(),
      shell_integration_shell_available
          ? mrb_str_new(mrb, shell_integration_shell,
                        (mrb_int)shell_integration_shell_length)
          : mrb_nil_value(),
      user_var_entries,
      pid_available ? mrb_int_value(mrb, (mrb_int)pid) : mrb_nil_value(),
      mrb_bool_value(command_running != 0),
      last_exit_status_available
          ? mrb_int_value(mrb, (mrb_int)last_exit_status)
          : mrb_nil_value(),
      mrb_str_new(mrb, screen_text, (mrb_int)screen_text_length),
      mrb_bool_value(zoomed != 0),
      icon_title_available
          ? mrb_str_new(mrb, icon_title, (mrb_int)icon_title_length)
          : mrb_nil_value(),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_pane"), 13, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

static mrb_value optional_integer(mrb_state *mrb, uint64_t value) {
  return value == UINT64_MAX ? mrb_nil_value()
                             : mrb_int_value(mrb, (mrb_int)value);
}

int toyoterm_mruby_emit_event(
    void *state, const char *name, size_t name_length, uint64_t workspace_id,
    uint64_t window_id, uint64_t tab_id, uint64_t pane_id, const char *title,
    size_t title_length, int title_available, const char *cwd, size_t cwd_length,
    int cwd_available, int exit_status, int exit_status_available,
    char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[8] = {
      mrb_str_new(mrb, name, (mrb_int)name_length),
      optional_integer(mrb, workspace_id),
      optional_integer(mrb, window_id),
      optional_integer(mrb, tab_id),
      optional_integer(mrb, pane_id),
      title_available ? mrb_str_new(mrb, title, (mrb_int)title_length)
                      : mrb_nil_value(),
      cwd_available ? mrb_str_new(mrb, cwd, (mrb_int)cwd_length)
                      : mrb_nil_value(),
      exit_status_available ? mrb_int_value(mrb, (mrb_int)exit_status)
                            : mrb_nil_value(),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__emit_native_event"), 8, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_set_clipboard_text(void *state, const char *text,
                                      size_t length, int available,
                                      char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value argument = available ? mrb_str_new(mrb, text, (mrb_int)length)
                                 : mrb_nil_value();
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__set_clipboard_text"), 1, &argument);
  return finish_arena_call(mrb, arena_index, error_output);
}

int toyoterm_mruby_eval(void *state, const char *source, const char *filename,
                        char **output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *output = NULL;
  mrb->exc = NULL;

  mrb_ccontext *context = mrb_ccontext_new(mrb);
  if (context == NULL) {
    mrb_gc_arena_restore(mrb, arena_index);
    return 2;
  }
  mrb_ccontext_filename(mrb, context, filename);
  mrb_value value = mrb_load_string_cxt(mrb, source, context);
  mrb_ccontext_free(mrb, context);
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb_gc_protect(mrb, exception);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    mrb_gc_arena_restore(mrb, arena_index);
    return 1;
  }

  value = mrb_obj_as_string(mrb, value);
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb_gc_protect(mrb, exception);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    mrb_gc_arena_restore(mrb, arena_index);
    return 1;
  }
  *output = copy_mruby_string(value);
  int status = *output == NULL ? 2 : 0;
  mrb_gc_arena_restore(mrb, arena_index);
  return status;
}

int toyoterm_mruby_invoke_async_callback(
    void *state, uint64_t id,
    const uint8_t *stdout_bytes, size_t stdout_length,
    const uint8_t *stderr_bytes, size_t stderr_length,
    int32_t exit_status, int32_t launch_error, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  int arena_index = mrb_gc_arena_save(mrb);
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[5] = {
      mrb_int_value(mrb, (mrb_int)id),
      mrb_str_new(mrb, stdout_bytes == NULL ? "" : (const char *)stdout_bytes, (mrb_int)stdout_length),
      mrb_str_new(mrb, stderr_bytes == NULL ? "" : (const char *)stderr_bytes, (mrb_int)stderr_length),
      mrb_int_value(mrb, (mrb_int)exit_status),
      mrb_bool_value(launch_error != 0),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__invoke_async_callback"), 5, arguments);
  return finish_arena_call(mrb, arena_index, error_output);
}

void toyoterm_mruby_gc_stats(void *state, size_t *arena_index,
                             size_t *live_objects) {
  mrb_state *mrb = (mrb_state *)state;
  *arena_index = (size_t)mrb->gc.arena_idx;
  *live_objects = mrb->gc.live;
}

void toyoterm_mruby_string_free(char *string) { free(string); }
