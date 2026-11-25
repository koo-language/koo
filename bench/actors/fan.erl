%% Sixty-four gen_servers are each told two hundred thousand things by sixteen others.
-module(fan).
-behaviour(gen_server).
-export([main/0, init/1, handle_call/3, handle_cast/2]).

init(_) -> {ok, 0}.
handle_cast({add, By}, Got) -> {noreply, Got + By};
handle_cast({pour, Sinks, Times}, Got) ->
    pour(Sinks, tuple_size(Sinks), 0, Times),
    {noreply, Got}.
handle_call(total, _From, Got) -> {reply, Got, Got};
handle_call(done, _From, Got) -> {reply, true, Got}.

pour(_, _, Times, Times) -> ok;
pour(Sinks, Count, I, Times) ->
    gen_server:cast(element(I rem Count + 1, Sinks), {add, 1}),
    pour(Sinks, Count, I + 1, Times).

main() ->
    Sinks = [element(2, gen_server:start(?MODULE, [], [])) || _ <- lists:seq(1, 64)],
    Sources = [element(2, gen_server:start(?MODULE, [], [])) || _ <- lists:seq(1, 16)],
    lists:foreach(fun(S) -> gen_server:cast(S, {pour, list_to_tuple(Sinks), 800000}) end, Sources),
    lists:foreach(fun(S) -> gen_server:call(S, done, infinity) end, Sources),
    Total = lists:sum([gen_server:call(S, total, infinity) || S <- Sinks]),
    io:format("~p~n", [Total]),
    halt().
