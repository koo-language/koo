%% A million gen_servers, each called once, and all of them kept.
-module(many).
-behaviour(gen_server).
-export([main/0, init/1, handle_call/3, handle_cast/2]).

init(Name) -> {ok, {0, Name}}.
handle_call({set, To}, _From, {_, Name}) -> {reply, To, {To, Name}};
handle_call(get, _From, {Value, Name}) -> {reply, Value, {Value, Name}}.
handle_cast(_, State) -> {noreply, State}.

start(N, N, Acc) -> Acc;
start(I, N, Acc) ->
    {ok, Pid} = gen_server:start(?MODULE, <<"cell">>, []),
    gen_server:call(Pid, {set, I}),
    start(I + 1, N, [Pid | Acc]).

main() ->
    Pids = start(0, 1000000, []),
    Total = lists:foldl(fun(Pid, Sum) -> Sum + gen_server:call(Pid, get) end, 0, Pids),
    io:format("~p actors, total ~p~n", [length(Pids), Total]),
    io:format("memory ~p MB~n", [erlang:memory(total) div 1048576]),
    halt().
