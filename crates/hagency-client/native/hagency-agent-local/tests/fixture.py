#!/usr/bin/env python3
# Offline stdio actor for the Desktop bridge; no provider or network calls.
import json, sys, pathlib
mode = __MODE__
kind = __KIND__
root = pathlib.Path(__ROOT__)

def send(value):
    print(json.dumps(value), flush=True)

def read():
    value = json.loads(sys.stdin.readline())
    with (root / 'frames').open('a') as out:
        out.write(json.dumps(value) + '\n')
    return value

if sys.argv[1:3] == ['auth', 'status']:
    send({'loggedIn': True})
    sys.exit(0)
with (root / 'argv').open('a') as out:
    out.write(json.dumps(sys.argv[1:]) + '\n')

if kind == 'claude':
    init = read()
    send({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': init['request_id'], 'response': {}}})
    read()
    session = 'offline-session'
    send({'type': 'system', 'subtype': 'init', 'session_id': session})
    if mode in ('approval', 'cancel', 'hold'):
        send({'type': 'assistant', 'session_id': session, 'message': {'id': 'step-1', 'content': [{'type': 'tool_use', 'id': 'tool-1', 'name': 'Write', 'input': {'file_path': str(root / 'outside' / 'result')}}], 'usage': {'input_tokens': 10, 'output_tokens': 2, 'cache_read_input_tokens': 3, 'cache_creation_input_tokens': 4}}})
        send({'type': 'control_request', 'request_id': 'approval-1', 'request': {'subtype': 'can_use_tool', 'tool_name': 'Write', 'tool_use_id': 'tool-1', 'input': {'file_path': str(root / 'outside' / 'result'), 'content': 'exact'}}})
        if mode == 'cancel':
            send({'type': 'control_cancel_request', 'request_id': 'approval-1'})
        else:
            response = read()['response']['response']
            (root / 'decision').write_text(json.dumps(response))
            if response['behavior'] == 'allow':
                (root / 'effect').write_text('one')
        send({'type': 'user', 'session_id': session, 'message': {'content': [{'type': 'tool_result', 'tool_use_id': 'tool-1', 'content': 'done'}]}})
    usage = {} if mode == 'unknown' else {'input_tokens': 10, 'output_tokens': 7, 'cache_read_input_tokens': 3, 'cache_creation_input_tokens': 4}
    send({'type': 'result', 'subtype': 'error' if mode == 'failed' else 'success', 'is_error': mode == 'failed', 'result': 'offline reply', 'session_id': session, 'usage': usage})
else:
    def answer(request, value):
        send({'jsonrpc': '2.0', 'id': request['id'], 'result': value})
    def notify(method, params):
        send({'jsonrpc': '2.0', 'method': method, 'params': params})
    request = read()
    answer(request, {'type': 'server_hello', 'capabilities': {'version': {'protocol': 'octos-ui/v1alpha1'}}})
    request = read()
    answer(request, {'current': {'mode': 'read_only', 'network': 'deny'}})
    request = read()
    session = request['params']['session_id']
    answer(request, {'opened': {'session_id': session, 'workspace_root': request['params']['cwd'], 'active_profile_id': 'coding'}})
    request = read()
    turn = request['params']['turn_id']
    answer(request, {'accepted': True})
    seq = 0
    def envelope(tag, data):
        global seq
        seq += 1
        notify('projection/envelope', {'session_id': session, 'thread_id': 'main', 'seq': seq, 'turn_id': turn, 'payload': {'type': tag, 'data': data}})
    if mode in ('approval', 'cancel', 'hold'):
        envelope('tool_start', {'tool_call_id': 'tool-1', 'name': 'shell'})
        notify('approval/requested', {'session_id': session, 'approval_id': 'approval-1', 'turn_id': turn, 'tool_name': 'shell', 'title': 'Run a command', 'body': 'offline', 'approval_kind': 'command', 'risk': 'high', 'typed_details': {'kind': 'command', 'command': {'command_line': 'offline', 'cwd': str(root / 'outside'), 'tool_call_id': 'tool-1'}}})
        if mode == 'cancel':
            notify('approval/cancelled', {'session_id': session, 'approval_id': 'approval-1'})
        else:
            response = read()
            (root / 'decision').write_text(json.dumps(response['params']))
            if response['params']['decision'] == 'approve':
                (root / 'effect').write_text('one')
            answer(response, {'accepted': True})
        envelope('tool_end', {'tool_call_id': 'tool-1'})
    envelope('assistant_persisted', {'text': 'offline reply'})
    tokens = {} if mode == 'unknown' else {'token_usage': {'input_tokens': 10, 'output_tokens': 7, 'cache_read_tokens': 3, 'cache_write_tokens': 4, 'reasoning_tokens': 2}}
    envelope('turn_terminal', {'outcome': 'errored' if mode == 'failed' else 'completed', **tokens})
    notify('session/orchestration', {'session_id': session, 'active': False})
    request = read()
    count_file = root / 'count'
    count = int(count_file.read_text()) + 1 if count_file.exists() else 1
    count_file.write_text(str(count))
    totals = {} if mode == 'unknown' or (mode == 'mixed_usage' and count == 1) else {'input_tokens': 10 * count, 'output_tokens': 7 * count, 'cached_input_tokens': 3 * count, 'cache_write_input_tokens': 4 * count}
    answer(request, {'usage': totals})
# Keep the pipe alive until the host drops/stops the original child.
sys.stdin.read()
