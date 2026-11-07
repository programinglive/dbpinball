<?php

namespace ProgrammingLive\LaravelSimpleBackup\Facades;

use Illuminate\Support\Facades\Facade;

class Backup extends Facade
{
    /**
     * Get the registered name of the component.
     */
    protected static function getFacadeAccessor(): string
    {
        return 'backup';
    }
}
