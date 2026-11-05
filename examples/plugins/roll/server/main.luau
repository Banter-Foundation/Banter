banter.events.on("command", function(event)
  if event.name ~= "roll" then
    return
  end
  local n = math.random(1, 6)
  banter.messages.send({
    channel_id = event.channel_id,
    body = "Rolled " .. tostring(n),
  })
end)
