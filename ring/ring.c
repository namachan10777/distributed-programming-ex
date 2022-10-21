#include "abt.h"
#include "margo-logging.h"
#include "mercury_macros.h"
#include "mercury_proc.h"
#include <assert.h>
#include <bits/pthreadtypes.h>
#include <linux/limits.h>
#include <margo.h>
#include <mercury.h>
#include <mercury_core_types.h>
#include <mercury_macros.h>
#include <mercury_proc_string.h>
#include <mercury_types.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// 2sに一回 coordinatorはlist rpcを発行する
// 3sの間list rpcを受け取らなかった場合はcoordinator不在と判断
// 0-3sの間のランダムな時間sleepしてlist rpcを独自に発行
// ノードのリストを集め終えたらcoordinator rpcを発行してcoordinatorを選出
// 本来は多分coordinator選出中は新たなlist
// rpcの発行を防ぐべきだが、3s以内には選出終わるだろうしヨシ!

static void join(hg_handle_t h);
static void set_next(hg_handle_t h);
static void set_prev(hg_handle_t h);
static void list(hg_handle_t h);
static void election(hg_handle_t h);
static void coordinate(hg_handle_t h);
DECLARE_MARGO_RPC_HANDLER(join)
DECLARE_MARGO_RPC_HANDLER(set_next)
DECLARE_MARGO_RPC_HANDLER(set_prev)
DECLARE_MARGO_RPC_HANDLER(list)
DECLARE_MARGO_RPC_HANDLER(election)
DECLARE_MARGO_RPC_HANDLER(coordinate)
static hg_return_t join_ring(hg_string_t _addr);
static void *heatbeater(void *param);

enum {
  RECIEVED,
  RESETED,
} coordinator_called_flag = RESETED;
ABT_mutex coordinator_called_flag_lock;

typedef struct {
  uint32_t len;
  hg_string_t *names;
} node_list_t;

int am_i_coordinator = 0;
ABT_mutex am_i_coordinator_lock;

static inline hg_return_t hg_proc_node_list_t(hg_proc_t proc, void *data) {
  node_list_t *list = data;
  hg_return_t ret;
  if ((ret = hg_proc_uint32_t(proc, &list->len)) != HG_SUCCESS) {
    return ret;
  }
  if (hg_proc_get_op(proc) == HG_FREE) {
    assert(list->names);
    free(list->names);
  } else if (hg_proc_get_op(proc) == HG_DECODE) {
    list->names = malloc(sizeof(hg_string_t) * list->len);
  }
  assert(list->names);
  for (size_t i = 0; i < list->len; ++i) {
    if ((ret = hg_proc_hg_string_t(proc, &list->names[i])) != HG_SUCCESS) {
      goto err;
    }
  }

  return HG_SUCCESS;
err:
  free(list->names);
  return ret;
}

MERCURY_GEN_PROC(coordinate_ring_t, ((node_list_t)(ring))((node_list_t)(read)))

ABT_mutex join_lock;

static struct {
  margo_instance_id mid;
  margo_instance_id client_mid;
  hg_addr_t me;
  hg_addr_t next;
  hg_addr_t prev;
  hg_id_t join_rpc;
  hg_id_t set_next_rpc;
  hg_id_t set_prev_rpc;
  hg_id_t coordinate_rpc;
  hg_id_t list_rpc;
  hg_id_t election_rpc;
} env;

hg_return_t lookup_addr(hg_addr_t *addr, hg_string_t name) {
  hg_return_t ret = margo_addr_lookup(env.mid, name, addr);
  if (ret != HG_SUCCESS) {
    fprintf(stderr, "not found %s (%s)\n", name, HG_Error_to_string(ret));
    // TOD
    return ret;
  }
  return HG_SUCCESS;
}

int main(int argc, char *argv[]) {
  hg_return_t ret;
  char addr_str[PATH_MAX];

  printf("initializing server\n");

  if (ABT_mutex_create(&join_lock) != 0) {
    return 1;
  }
  if (ABT_mutex_create(&coordinator_called_flag_lock) != 0) {
    return 1;
  }
  if (ABT_mutex_create(&am_i_coordinator_lock) != 0) {
    return 1;
  }

  env.mid = margo_init("tcp", MARGO_SERVER_MODE, 1, 10);
  assert(env.mid);

  if ((ret = margo_addr_self(env.mid, &env.me)) != HG_SUCCESS) {
    goto err;
  };
  size_t my_address_size = sizeof(addr_str);
  if ((ret = margo_addr_to_string(env.mid, addr_str, &my_address_size,
                                  env.me)) != HG_SUCCESS) {
    goto err;
  }

  printf("setting rpc handlers\n");
  // TODO: error handling
  // join rpc handler
  env.join_rpc =
      MARGO_REGISTER(env.mid, "join", hg_string_t, hg_string_t, join);

  // set_next rpc handler
  env.set_next_rpc =
      MARGO_REGISTER(env.mid, "set_next", hg_string_t, void, set_next);
  if ((ret = margo_registered_disable_response(env.mid, env.set_next_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }
  // set_prev rpc handler
  env.set_prev_rpc =
      MARGO_REGISTER(env.mid, "set_prev", hg_string_t, void, set_prev);
  if ((ret = margo_registered_disable_response(env.mid, env.set_prev_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }

  // set list rpc handler
  env.list_rpc = MARGO_REGISTER(env.mid, "list", node_list_t, void, list);
  if ((ret = margo_registered_disable_response(env.mid, env.list_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }

  // set election rpc handler
  env.election_rpc =
      MARGO_REGISTER(env.mid, "election", node_list_t, void, election);
  if ((ret = margo_registered_disable_response(env.mid, env.election_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }

  // coordinate rpc handler
  env.coordinate_rpc = MARGO_REGISTER(env.mid, "coordinate", coordinate_ring_t,
                                      void, coordinate);
  if ((ret = margo_registered_disable_response(env.mid, env.coordinate_rpc,
                                               HG_TRUE)) != HG_SUCCESS) {
    goto err;
  }

  printf("initializing address\n");
  // ring init
  // nextとprevを自分自身に向ける
  if ((ret = lookup_addr(&env.next, addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.prev, addr_str)) != HG_SUCCESS) {
    goto err;
  }

  if (argc > 1) {
    if ((ret = join_ring(argv[1])) != HG_SUCCESS) {
      goto err;
    }
  }

  pthread_t heatbeater_thread;

  if (pthread_create(&heatbeater_thread, NULL, heatbeater, NULL) != 0) {
    exit(1);
  }

  printf("my addr: %s\n", addr_str);

  margo_wait_for_finalize(env.mid);
  if (pthread_detach(heatbeater_thread) != 0) {
    exit(1);
  }
  return 0;
err:
  fprintf(stderr, "main: %s\n", HG_Error_to_string(ret));
  return 1;
}

static hg_return_t join_ring(hg_string_t next) {
  hg_return_t ret;

  // self.next = next
  if ((ret = lookup_addr(&env.next, next)) != HG_SUCCESS) {
    return ret;
  }

  hg_handle_t h;
  if ((ret = margo_create(env.mid, env.next, env.join_rpc, &h)) != HG_SUCCESS) {
    return ret;
  }

  char my_addr_str[PATH_MAX];
  size_t my_addr_str_size = sizeof(my_addr_str);
  if ((ret = margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size,
                                  env.me)) != HG_SUCCESS) {
    goto err;
  }
  printf("my_addr: %s\n", my_addr_str);
  char *my_addr_str_ptr = (char *)my_addr_str;
  if ((ret = margo_forward(h, &my_addr_str_ptr)) != HG_SUCCESS) {
    goto err;
  }

  char *prev_addr_str;
  if ((ret = margo_get_output(h, &prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }

  printf("join succeeded: next.prev = %s\n", prev_addr_str);

  assert(margo_destroy(h) == HG_SUCCESS);
  // self.prev = next.prev
  return lookup_addr(&env.prev, prev_addr_str);
err:
  margo_destroy(h);
  fprintf(stderr, "join_ring: %s\n", HG_Error_to_string(ret));
  return ret;
}

static void join(hg_handle_t h) {
  ABT_mutex_lock(join_lock);
  hg_return_t ret;
  char *joined_addr_str;
  printf("enter join_rcp handler\n");
  if ((ret = margo_get_input(h, &joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  printf("recieved joining address: %s\n", joined_addr_str);
  fflush(stdout);

  // prev.set_next('n)
  hg_handle_t prev_h;
  if ((ret = margo_create(env.mid, env.prev, env.set_next_rpc, &prev_h)) !=
      HG_SUCCESS) {
    goto err_prev;
  }
  if ((ret = margo_forward(prev_h, &joined_addr_str)) != HG_SUCCESS) {
    goto err_prev;
  }
  if ((ret = margo_destroy(prev_h)) != HG_SUCCESS) {
    goto err;
  }

  // return old_prev
  char old_prev[PATH_MAX];
  size_t old_prev_size = sizeof(old_prev);
  if ((ret = margo_addr_to_string(env.mid, old_prev, &old_prev_size,
                                  env.prev)) != HG_SUCCESS) {
    goto err;
  }
  char *old_prev_ptr = (char *)old_prev;
  printf("response %s\n", old_prev);
  if ((ret = margo_respond(h, &old_prev_ptr)) != HG_SUCCESS) {
    goto err;
  }

  // self.prev = n'
  if ((ret = lookup_addr(&env.prev, joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &joined_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  ABT_mutex_unlock(join_lock);
  return;
err_prev:
  margo_destroy(prev_h);
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "join_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}
static void set_prev(hg_handle_t h) {
  hg_return_t ret;
  char *prev_addr_str;
  if ((ret = margo_get_input(h, &prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.prev, prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &prev_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "set_prev_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}
static void set_next(hg_handle_t h) {
  hg_return_t ret;
  char *next_addr_str;
  if ((ret = margo_get_input(h, &next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = lookup_addr(&env.next, next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_free_input(h, &next_addr_str)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "set_next_rpc handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}

node_list_t push_name(node_list_t node_list, hg_string_t name) {
  node_list_t ret;
  ret.len = node_list.len + 1;
  ret.names = malloc(sizeof(hg_string_t) * ret.len);
  for (size_t i = 0; i < node_list.len; ++i) {
    size_t name_mem_size = sizeof(char) * (1 + strlen(node_list.names[i]));
    ret.names[i] = malloc(name_mem_size);
    memcpy(ret.names[i], node_list.names[i], name_mem_size);
  }
  size_t name_mem_size = sizeof(char) * (1 + strlen(name));
  ret.names[node_list.len] = malloc(name_mem_size);
  memcpy(ret.names[node_list.len], name, name_mem_size);

  return ret;
}

node_list_t push_my_name(node_list_t node_list) {
  char my_addr_str[PATH_MAX];
  size_t my_addr_str_size = sizeof(my_addr_str);
  if (margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size, env.me) !=
      HG_SUCCESS) {
    exit(1);
  }
  return push_name(node_list, my_addr_str);
}

node_list_t empty_node_list() {
  node_list_t ret = {
      .len = 0,
      .names = NULL,
  };
  return ret;
}

int node_list_include_my_name(node_list_t list) {
  char my_addr_str[PATH_MAX];
  size_t my_addr_str_size = sizeof(my_addr_str);
  hg_return_t ret;
  if ((ret = margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size,
                                  env.me)) != HG_SUCCESS) {
    exit(1);
  }
  for (size_t i = 0; i < list.len; ++i) {
    if (strcmp(list.names[i], my_addr_str) == 0) {
      return 1;
    }
  }
  return 0;
}

int check_am_i_coordinator(node_list_t list) {
  char my_addr_str[PATH_MAX];
  size_t my_addr_str_size = sizeof(my_addr_str);
  if (margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size, env.me) !=
      HG_SUCCESS) {
    exit(1);
  }
  for (size_t i = 0; i < list.len; ++i) {
    if (strcmp(my_addr_str, list.names[i]) > 0) {
      return 0;
    }
  }
  return 1;
}

void *heatbeater(void *param) {
  hg_handle_t h;
  hg_return_t ret;

  if (param != NULL) {
    exit(1);
  }

  for (;;) {
    if (am_i_coordinator) {
      printf("I am coordinator\n");
      if ((ret = margo_create(env.mid, env.prev, env.list_rpc, &h))) {
        goto exit;
      }
      node_list_t initial = push_my_name(empty_node_list());
      if ((ret = margo_forward(h, &initial)) != HG_SUCCESS) {
        goto err;
      }
      if ((ret = margo_destroy(h)) != HG_SUCCESS) {
        goto exit;
      }
      margo_thread_sleep(env.mid, 2000);
    } else {
      ABT_mutex_lock(coordinator_called_flag_lock);
      if (coordinator_called_flag == RESETED) {
        ABT_mutex_unlock(coordinator_called_flag_lock);

        // 3sの間スリープ
        // coordinatorが生きていればこの間にlist rpcが来るはず
        margo_thread_sleep(env.mid, 3000);

        ABT_mutex_lock(coordinator_called_flag_lock);
        if (coordinator_called_flag == RESETED) {
          ABT_mutex_unlock(coordinator_called_flag_lock);

          printf("Coordinator has gone!\n");

          // coordinator不在と判断
          hg_handle_t h;
          hg_return_t ret;
          if ((ret = margo_create(env.mid, env.prev, env.election_rpc, &h)) !=
              HG_SUCCESS) {
            goto exit;
          }

          node_list_t initial_list = push_my_name(empty_node_list());
          if ((ret = margo_forward(h, &initial_list)) != HG_SUCCESS) {
            goto err;
          }
          if ((ret = margo_destroy(h) != HG_SUCCESS)) {
            goto exit;
          }
          continue;

        } else {
          printf("Coordinator elected.\n");
          coordinator_called_flag = RESETED;
          ABT_mutex_unlock(coordinator_called_flag_lock);
        }
      } else {
        printf("Coordinator exists.\n");
        coordinator_called_flag = RESETED;
        ABT_mutex_unlock(coordinator_called_flag_lock);
      }

      // 1s毎のチェック
      margo_thread_sleep(env.mid, 1000);
    }
  }
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "heatbeater: %s\n", HG_Error_to_string(ret));
  exit(1);
  return NULL;
}

static void coordinate(hg_handle_t h) {
  hg_return_t ret;
  coordinate_ring_t ring;
  if ((ret = margo_get_input(h, &ring)) != HG_SUCCESS) {
    goto err;
  }

  ABT_mutex_lock(am_i_coordinator_lock);
  am_i_coordinator = check_am_i_coordinator(ring.ring);
  ABT_mutex_unlock(am_i_coordinator_lock);

  if (node_list_include_my_name(ring.read)) {
    return;
  }

  ring.read = push_my_name(ring.read);

  hg_handle_t prev_h;
  if ((ret = margo_create(env.mid, env.prev, env.coordinate_rpc, &prev_h)) !=
      HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_forward(prev_h, &ring)) != HG_SUCCESS) {
    goto err_prev;
  }
  if ((ret = margo_destroy(prev_h)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;

err_prev:
  margo_destroy(prev_h);
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "coordinate: %s\n", HG_Error_to_string(ret));
  exit(1);
}

static void election(hg_handle_t h) {
  hg_return_t ret;
  node_list_t node_list;
  printf("recieve election rpc\n");
  if ((ret = margo_get_input(h, &node_list)) != HG_SUCCESS) {
    goto err;
  }

  hg_handle_t prev_h;
  if (node_list_include_my_name(node_list)) {
    // 自分自身が発行したlist rpc
    for (size_t i = 0; i < node_list.len; ++i) {
      printf("eligibility member[%ld]: %s\n", i, node_list.names[i]);
    }
    if ((ret = margo_create(env.mid, env.prev, env.coordinate_rpc, &prev_h)) !=
        HG_SUCCESS) {
      goto err_prev;
    }
    node_list_t read_list = push_my_name(empty_node_list());
    coordinate_ring_t ring = {
        .read = read_list,
        .ring = node_list,
    };
    if ((ret = margo_forward(prev_h, &ring)) != HG_SUCCESS) {
      goto err_prev;
    }

    ABT_mutex_lock(am_i_coordinator_lock);
    am_i_coordinator = check_am_i_coordinator(node_list);
    ABT_mutex_unlock(am_i_coordinator_lock);
  } else {
    node_list_t updated_node_list = push_my_name(node_list);

    if ((ret = margo_create(env.mid, env.prev, env.list_rpc, &prev_h)) !=
        HG_SUCCESS) {
      goto err;
    }

    if ((ret = margo_forward(prev_h, &updated_node_list)) != HG_SUCCESS) {
      goto err_prev;
    }
  }

  if ((ret = margo_destroy(prev_h)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;

err_prev:
  margo_destroy(prev_h);
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "list handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}

static void list(hg_handle_t h) {
  hg_return_t ret;
  node_list_t node_list;
  printf("recieve list rpc\n");
  if ((ret = margo_get_input(h, &node_list)) != HG_SUCCESS) {
    goto err;
  }

  ABT_mutex_lock(coordinator_called_flag_lock);
  coordinator_called_flag = RECIEVED;
  ABT_mutex_unlock(coordinator_called_flag_lock);

  hg_handle_t prev_h;
  if (node_list_include_my_name(node_list)) {
    // 自分自身が発行したlist rpc
    for (size_t i = 0; i < node_list.len; ++i) {
      printf("member[%ld]: %s\n", i, node_list.names[i]);
    }

    if ((ret = margo_destroy(h)) != HG_SUCCESS) {
      goto err;
    }
    return;
  } else {
    char my_addr_str[PATH_MAX];
    size_t my_addr_str_size = sizeof(my_addr_str);
    if ((ret = margo_addr_to_string(env.mid, my_addr_str, &my_addr_str_size,
                                    env.me)) != HG_SUCCESS) {
      goto err;
    }

    node_list_t updated_node_list = push_name(node_list, my_addr_str);

    if ((ret = margo_create(env.mid, env.prev, env.list_rpc, &prev_h)) !=
        HG_SUCCESS) {
      goto err;
    }

    if ((ret = margo_forward(prev_h, &updated_node_list)) != HG_SUCCESS) {
      goto err_prev;
    }
  }

  if ((ret = margo_destroy(prev_h)) != HG_SUCCESS) {
    goto err;
  }
  if ((ret = margo_destroy(h)) != HG_SUCCESS) {
    goto exit;
  }
  return;

err_prev:
  margo_destroy(prev_h);
err:
  margo_destroy(h);
exit:
  fprintf(stderr, "list handler: %s\n", HG_Error_to_string(ret));
  exit(1);
}

DEFINE_MARGO_RPC_HANDLER(join)
DEFINE_MARGO_RPC_HANDLER(set_prev)
DEFINE_MARGO_RPC_HANDLER(set_next)
DEFINE_MARGO_RPC_HANDLER(list)
DEFINE_MARGO_RPC_HANDLER(coordinate)
DEFINE_MARGO_RPC_HANDLER(election)
