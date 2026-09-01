#include <stdlib.h>
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
