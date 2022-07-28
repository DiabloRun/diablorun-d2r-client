## Download LiveSplit and Server Component
* [Download LiveSplit](http://livesplit.org/) 
* Download [Server Component](https://github.com/LiveSplit/LiveSplit.Server/releases/download/1.8.19/LiveSplit.Server.zip)
* Copy LiveSplit.Server.dll and Noesis.Javascript.dll to your LiveSplit/Components folder

## Setup LiveSplit
* Open LiveSplit and enter edit layout window
* Add > Control > LiveSplit Server
* Double click on LiveSplit server and make note of the IP and port
* Save layout and make sure you have a timer that tracks "game time"
* Right click LiveSplit window and click Control > Start server
* Note that you need to Start server every time you open LiveSplit

## Download the diablo.run client
This tool communicates with LiveSplit server component and tracks load states.

* [Download latest release](https://github.com/DiabloRun/diablorun-d2r-client/releases)
* Run diablo.run.Setup.exe
* Diablo.run should open after installation is complete
* Check "Enable LiveSplit client"
* Paste your LiveSplit server IP and Port (For example 192.168.0.181:16834)
* Connection with LiveSplit was successful if you see "connected"
* Open D2R and test if it's working
* If everything worked correctly then you should see the Diablo.run client swap between "playing" and "loading"

## Twitch extension to display character gear
* [Install Diablo.run Armory for D2R](https://dashboard.twitch.tv/extensions/n1xuo058lazw6dskgsp37y6zw4xuus-0.0.7) Twitch extension
* Get your API key from [diablo.run/setup](https://diablo.run/setup)
* Add your API key to the Diablo.run client
* Browse for your saved games directory for D2R saves folder. Usually in C:\Users\Saved Games\Diablo II Resurrected
* Diablo.run client reads the save files to send character data to the extension
* Your gear will be visible to viewers through the extension, but also with the diablo.run link. For example: [diablo.run/Indrek/@](https://diablo.run/Indrek/@)
