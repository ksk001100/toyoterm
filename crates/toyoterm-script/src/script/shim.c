#include <stdlib.h>
#include <stdint.h>
#include <string.h>

#include "mruby.h"

extern int toyoterm_host_read_file(const uint8_t *path, size_t path_length,
                                   uint8_t **output, size_t *output_length,
                                   char **error);
extern int toyoterm_host_spawn(const uint8_t *const *arguments,
                               const size_t *lengths, size_t count,
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

static int finish_typed_call(mrb_state *mrb, char **error_output) {
  if (mrb->exc == NULL) {
    return 0;
  }
  mrb_value exception = mrb_obj_value(mrb->exc);
  mrb->exc = NULL;
  mrb_value error = format_exception(mrb, exception);
  *error_output = copy_mruby_string(error);
  mrb->exc = NULL;
  return *error_output == NULL ? 2 : 1;
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
  mrb_get_args(mrb, "A", &arguments);
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
  int status = toyoterm_host_spawn(
      pointers, lengths, (size_t)count, &stdout_output, &stdout_length,
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
  struct RClass *module = mrb_module_get(mrb, "Toyoterm");
  mrb_define_module_function(mrb, module, "__host_read_file", host_read_file,
                             MRB_ARGS_REQ(1));
  mrb_define_module_function(mrb, module, "__host_spawn", host_spawn,
                             MRB_ARGS_REQ(1));
}

int toyoterm_mruby_set_environment(void *state, const char *const *keys,
                                   const char *const *values,
                                   const size_t *lengths, size_t count,
                                   char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
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
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_set_current_pane(void *state, uint64_t pane_id,
                                    char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value argument = mrb_int_value(mrb, (mrb_int)pane_id);
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__set_current_pane"), 1, &argument);
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_set_live_handles(
    void *state, const uint64_t *workspaces, size_t workspace_count,
    const uint64_t *windows, size_t window_count, const uint64_t *tabs,
    size_t tab_count, const uint64_t *panes, size_t pane_count,
    char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
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
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_reset_object_model(void *state, uint64_t workspace_id,
                                      uint64_t window_id, uint64_t tab_id,
                                      uint64_t pane_id, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
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
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_add_workspace(void *state, uint64_t workspace_id,
                                 const char *name, size_t name_length,
                                 const uint64_t *windows, size_t window_count,
                                 char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[3] = {
      mrb_int_value(mrb, (mrb_int)workspace_id),
      mrb_str_new(mrb, name, (mrb_int)name_length),
      integer_array(mrb, windows, window_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_workspace"), 3, arguments);
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_add_window(void *state, uint64_t window_id,
                              const uint64_t *tabs, size_t tab_count,
                              char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[2] = {
      mrb_int_value(mrb, (mrb_int)window_id),
      integer_array(mrb, tabs, tab_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_window"), 2, arguments);
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_add_tab(void *state, uint64_t tab_id, const char *title,
                           size_t title_length, const uint64_t *panes,
                           size_t pane_count, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[3] = {
      mrb_int_value(mrb, (mrb_int)tab_id),
      mrb_str_new(mrb, title, (mrb_int)title_length),
      integer_array(mrb, panes, pane_count),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_tab"), 3, arguments);
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_add_pane(void *state, uint64_t pane_id, const char *title,
                            size_t title_length, const char *cwd,
                            size_t cwd_length, int cwd_available, uint64_t pid,
                            int pid_available, int command_running,
                            int32_t last_exit_status,
                            int last_exit_status_available,
                            char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[6] = {
      mrb_int_value(mrb, (mrb_int)pane_id),
      mrb_str_new(mrb, title, (mrb_int)title_length),
      cwd_available ? mrb_str_new(mrb, cwd, (mrb_int)cwd_length)
                    : mrb_nil_value(),
      pid_available ? mrb_int_value(mrb, (mrb_int)pid) : mrb_nil_value(),
      mrb_bool_value(command_running != 0),
      last_exit_status_available
          ? mrb_int_value(mrb, (mrb_int)last_exit_status)
          : mrb_nil_value(),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__add_pane"), 6, arguments);
  return finish_typed_call(mrb, error_output);
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
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_set_clipboard_text(void *state, const char *text,
                                      size_t length, int available,
                                      char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value argument = available ? mrb_str_new(mrb, text, (mrb_int)length)
                                 : mrb_nil_value();
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__set_clipboard_text"), 1, &argument);
  return finish_typed_call(mrb, error_output);
}

int toyoterm_mruby_eval(void *state, const char *source, const char *filename,
                        char **output) {
  mrb_state *mrb = (mrb_state *)state;
  *output = NULL;
  mrb->exc = NULL;

  mrb_ccontext *context = mrb_ccontext_new(mrb);
  if (context == NULL) {
    return 2;
  }
  mrb_ccontext_filename(mrb, context, filename);
  mrb_value value = mrb_load_string_cxt(mrb, source, context);
  mrb_ccontext_free(mrb, context);
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    return 1;
  }

  value = mrb_obj_as_string(mrb, value);
  if (mrb->exc != NULL) {
    mrb_value exception = mrb_obj_value(mrb->exc);
    mrb->exc = NULL;
    mrb_value error = format_exception(mrb, exception);
    *output = copy_mruby_string(error);
    mrb->exc = NULL;
    return 1;
  }
  *output = copy_mruby_string(value);
  return *output == NULL ? 2 : 0;
}

void toyoterm_mruby_string_free(char *string) { free(string); }
