#include <stdlib.h>
#include <stdint.h>
#include <string.h>

#include "mruby.h"

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
    int cwd_available, char **error_output) {
  mrb_state *mrb = (mrb_state *)state;
  *error_output = NULL;
  mrb->exc = NULL;
  mrb_value arguments[7] = {
      mrb_str_new(mrb, name, (mrb_int)name_length),
      optional_integer(mrb, workspace_id),
      optional_integer(mrb, window_id),
      optional_integer(mrb, tab_id),
      optional_integer(mrb, pane_id),
      title_available ? mrb_str_new(mrb, title, (mrb_int)title_length)
                      : mrb_nil_value(),
      cwd_available ? mrb_str_new(mrb, cwd, (mrb_int)cwd_length)
                    : mrb_nil_value(),
  };
  mrb_funcall_argv(mrb, toyoterm_module(mrb),
                   mrb_intern_lit(mrb, "__emit_native_event"), 7, arguments);
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
